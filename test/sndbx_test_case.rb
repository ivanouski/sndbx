# frozen_string_literal: true

require_relative 'test_helper'

# Runs sndbx with a fake docker that records its arguments and knows the images, the users they run as,
# and the containers the tests add
class SndbxTestCase < Minitest::Test
  include SndbxTestHelper

  TERMINAL_ENV = %w[--env=COLORTERM --env=TERM_PROGRAM --env=TERM_PROGRAM_VERSION].freeze
  CONTAINER_NAMES = ['ps', '--all', '--format', '{{.Names}}'].freeze
  # The uid and gid base is built with, the host user's on Linux unless it's root, which the fake docker's daemon
  # doesn't map
  HOST_IDS = ([Process.euid, Process.egid] if RbConfig::CONFIG['host_os'].include?('linux') && !Process.euid.zero?)

  FAKE_DOCKER = <<~RUBY
    #!#{RbConfig.ruby} --disable-gems
    require 'json'

    File.write(ENV.fetch('DOCKER_LOG'), "\#{ARGV.to_json}\\n", mode: 'a')
    images_path = ENV.fetch('DOCKER_IMAGES')
    images = File.exist?(images_path) ? File.readlines(images_path, chomp: true).to_h { |line| [line[/\\S+/], line[/ (.*)/, 1].to_s] } : {}
    containers_path = ENV.fetch('DOCKER_CONTAINERS')
    containers = File.exist?(containers_path) ? File.readlines(containers_path, chomp: true).map(&:split) : []
    networks_path = ENV.fetch('DOCKER_NETWORKS')
    networks = File.exist?(networks_path) ? File.readlines(networks_path, chomp: true).to_h(&:split) : {}
    proxy_path = ENV.fetch('DOCKER_PROXY')
    # What sndbx top reads: JSON objects by container name or tag, merged into the fake's own, and the proxy's log
    read_json = ->(name) { File.exist?(ENV.fetch(name, '')) ? JSON.parse(File.read(ENV.fetch(name))) : {} }
    merge = ->(base, extra) { base.merge(extra) { |_key, a, b| a.is_a?(Hash) && b.is_a?(Hash) ? merge.(a, b) : b } }
    access_log = ENV.fetch('DOCKER_ACCESS_LOG', '')

    case ARGV
    in ['info']
      exit(!ENV.key?('DOCKER_STOPPED'))
    in ['info', '--format', '{{.SecurityOptions}}']
      puts ENV.fetch('DOCKER_SECURITY_OPTIONS', '[name=seccomp,profile=builtin name=cgroupns]')
    in ['image', 'inspect', tag]
      exit(images.key?(tag))
    in ['image', 'inspect', '--format', '{{.Config.User}}', tag]
      exit(false) unless images.key?(tag)
      puts images[tag]
    in ['ps', *filters, '--format', '{{.Names}}']
      status = filters.find { |arg| arg.start_with?('status=') }&.delete_prefix('status=')
      status ||= 'running' unless filters.include?('--all')
      ancestor = filters.find { |arg| arg.start_with?('ancestor=') }&.delete_prefix('ancestor=')
      label = filters.find { |arg| arg.start_with?('label=') }
      containers.each do |image, name, state = 'running', tag = "sndbx:\#{image}"|
        next if status && state != status
        next if ancestor ? tag != ancestor : label && label != "label=sndbx.image=\#{image}"

        puts name
      end
    in ['image', 'rm', tag]
      exit(false) if ENV['DOCKER_BROKEN'] == tag || !images.key?(tag)
      File.write(images_path, images.except(tag).map { |name, user| "\#{name} \#{user}\\n" }.join)
    in ['network', 'inspect', '--format', '{{.Internal}}', name]
      exit(false) unless networks.key?(name)
      puts networks[name]
    in ['network', 'create', '--internal', name]
      File.write(networks_path, "\#{name} true\\n", mode: 'a')
      puts 'cafe' * 16
    in ['container', 'inspect', '--format', _, 'sndbx-proxy']
      exit(false) unless File.exist?(proxy_path)
      running, restarting, image, mount, *networks = File.read(proxy_path).split
      puts running, restarting, image, networks.join(' '), mount
    in ['run', '-d', '--name', 'sndbx-proxy', *]
      exit(false) if ENV['DOCKER_BROKEN'] == 'sndbx-proxy'
      mount = ARGV[ARGV.index('-v') + 1].split(':').first
      File.write(proxy_path, "true false \#{ARGV.last} \#{mount} bridge")
      puts 'f00d' * 16
    in ['network', 'connect', network, 'sndbx-proxy']
      exit(false) if ENV['DOCKER_BROKEN'] == network
      File.write(proxy_path, "\#{File.read(proxy_path)} \#{network}")
    in ['exec', 'sndbx-proxy', 'squid', '-k', 'reconfigure']
      exit(false) if ENV['DOCKER_BROKEN'] == 'reconfigure'
    in ['exec', 'sndbx-proxy', 'squid', '-k', 'parse'] | ['run', '--rm', '--entrypoint=squid', *, '-k', 'parse']
      if ENV['DOCKER_BROKEN'] == 'parse'
        warn '2026/10/22 10:04:58| Processing: http_port 3128', '2026/10/22 10:04:58| FATAL: Bungled squid.conf line 4'
        exit(false)
      end
    in ['rm', '-f', 'sndbx-proxy']
      File.delete(proxy_path)
    in ['pull', image]
      exit(false) if ENV['DOCKER_BROKEN'] == 'pull'
      File.write(images_path, "\#{image}\\n", mode: 'a')
    in ['build', *, '-t', tag, _]
      exit(false) if ENV['DOCKER_BROKEN'] == tag
      File.write(images_path, "\#{tag}\\n", mode: 'a')
    in ['ps', '--all', '--filter', 'label=sndbx.image', '--format', '{{.ID}}']
      containers.each { |image, name| puts name unless image == '-' }
    in ['inspect', *names]
      details = read_json.('DOCKER_INSPECT')
      found = names.filter_map do |name|
        image, _, state = containers.find { |container| container[1] == name }
        next unless image

        merge.({ 'Name' => "/\#{name}", 'Config' => { 'Labels' => { 'sndbx.image' => image } },
                 'State' => { 'Status' => state || 'running' }, 'NetworkSettings' => { 'Networks' => {} } },
               details.fetch(name, {}))
      end
      puts JSON.generate(found)
      exit(found.size == names.size)
    in ['stats', '--no-stream', '--format', '{{json .}}', *names]
      stats = read_json.('DOCKER_STATS')
      names.each { |name| puts JSON.generate({ 'Name' => name, **stats[name] }) if stats.key?(name) }
    in ['image', 'ls', '--format', '{{json .}}']
      details = read_json.('DOCKER_IMAGE_DETAILS')
      images.each_key do |tag|
        repository, _, name = tag.rpartition(':')
        puts JSON.generate({ 'Repository' => repository, 'Tag' => name, 'Size' => '1GB',
                             'CreatedAt' => '2026-01-01 00:00:00 +0000 UTC', **details.fetch(tag, {}) })
      end
    in ['exec', 'sndbx-proxy', 'tail', '-n', _, '/var/log/squid/access.log']
      print(File.read(access_log)) if File.exist?(access_log)
    in ['exec', '-i', 'sndbx-proxy', 'sh', '-c', _, 'sh', _, '/var/log/squid/access.log']
      print(File.read(access_log)) if File.exist?(access_log)
      $stdout.flush
      $stdin.read
    in ['events', *]
      # Like docker's, which doesn't read its stdin: only a signal ends it, or its next event once its reader is gone
      File.write(ENV.fetch('DOCKER_EVENTS_PID'), Process.pid.to_s) if ENV.key?('DOCKER_EVENTS_PID')
      sleep
    in ['stop' | 'rm', *, name]
      File.write(containers_path, containers.reject { |container| container[1] == name }.map { |c| "\#{c.join(' ')}\\n" }.join)
    else
    end
  RUBY

  def setup
    super
    @no_bin = mkdir('no-bin')
    @docker_log = File.join(@root, 'docker.log')
    @docker_images = File.join(@root, 'docker-images')
    @docker_containers = File.join(@root, 'docker-containers')
    @docker_networks = File.join(@root, 'docker-networks')
    @docker_proxy = File.join(@root, 'docker-proxy')
    @docker_inspect = File.join(@root, 'docker-inspect.json')
    @docker_stats = File.join(@root, 'docker-stats.json')
    @docker_image_details = File.join(@root, 'docker-image-details.json')
    @docker_access_log = File.join(@root, 'docker-access.log')
    @docker_events_pid = File.join(@root, 'docker-events.pid')

    write_executable(File.join(@bin, 'docker'), FAKE_DOCKER)
  end

  private

  def sndbx(*args, docker: true, env: {})
    super(*args, env: {
      'PATH' => docker ? @bin : @no_bin,
      'DOCKER_LOG' => @docker_log,
      'DOCKER_IMAGES' => @docker_images,
      'DOCKER_CONTAINERS' => @docker_containers,
      'DOCKER_NETWORKS' => @docker_networks,
      'DOCKER_PROXY' => @docker_proxy,
      'DOCKER_INSPECT' => @docker_inspect,
      'DOCKER_STATS' => @docker_stats,
      'DOCKER_IMAGE_DETAILS' => @docker_image_details,
      'DOCKER_ACCESS_LOG' => @docker_access_log
    }.merge(env))
  end

  def assert_error(message, result)
    refute_predicate result, :success?
    assert_equal "Error: #{message}.\n", result.err
  end

  # A settings error from the YAML parser, which ends with where it is in the file
  def assert_settings_error(message, result)
    refute_predicate result, :success?
    prefix = "Error: invalid #{settings_path}: #{message}"
    assert_match(/\A#{Regexp.escape(prefix)} at line \d+ column \d+\.\n\z/, result.err)
  end

  # An error clap found in the arguments, before sndbx does anything
  def assert_usage_error(message, result)
    assert_equal 2, result.status.exitstatus
    assert result.err.start_with?("error: #{message}"), result.err
  end

  def assert_help(usage, result)
    assert_predicate result, :success?
    assert_equal "#{usage}\n", result.out.lines.first
    refute File.exist?(settings_path)
    refute File.exist?(@docker_log)
  end

  def write_claude_settings
    write_settings(images: [{ name: 'claude' }])
    add_docker_image('sndbx:claude')
  end

  def write_build_settings(images_dir: @images_dir, **settings)
    write_dockerfile('base', "FROM ubuntu\n")
    write_dockerfile('claude', "ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n")
    write_dockerfile('plain', "FROM ubuntu\n")
    write_settings(images_dir: images_dir, **settings)
  end

  def write_exec_settings
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 'ubuntu' }])
    add_docker_container('claude', 'gifted_mcnulty')
    add_docker_container('codex', 'eager_turing')
    add_docker_container('claude', 'quirky_hopper')
  end

  def write_resume_settings
    write_exec_settings
    add_docker_container('claude', 'sleepy_babbage', 'exited')
  end

  # A container of the image in the settings, or of no image for one sndbx didn't start, from its tag
  def add_docker_container(image, name, state = 'running', tag = "sndbx:#{image}")
    File.write(@docker_containers, "#{image} #{name} #{state} #{tag}\n", mode: 'a')
  end

  def add_docker_image(tag, user: 'ubuntu')
    File.write(@docker_images, "#{tag} #{user}\n", mode: 'a')
  end

  def configured_images
    YAML.safe_load_file(settings_path)['images'].map { |image| image['name'] }
  end

  def docker_runs
    docker_calls.select { |call| call.first == 'run' }
  end

  def docker_execs
    docker_calls.select { |call| call.first == 'exec' }
  end

  def docker_starts
    docker_calls.select { |call| call.first == 'start' }
  end

  def docker_image_rms
    docker_calls.select { |call| call.first(2) == %w[image rm] }
  end

  def docker_builds
    docker_calls.select { |call| call.first == 'build' }
  end

  def run_options(workdir, name: File.basename(workdir))
    ['run', '--rm', '-it', '--label=sndbx.image=claude', *("--name=#{name}" if name), "--workdir=/workspace/#{workdir}"]
  end

  def volume(name, path = @work)
    ['-v', "#{path}:/workspace/#{name}"]
  end

  def config_volume
    ['-v', "#{default_config_dir}:/home/ubuntu/.claude"]
  end

  def default_config_dir
    File.join(@home, '.config', 'sndbx', 'claude')
  end

  # Base with `ids`, the host user's by default
  def build_call(name, base: false, prefix: 'sndbx', ids: HOST_IDS)
    args = base ? ['--build-arg', "TAG_PREFIX=#{prefix}"] : []
    args += ['--build-arg', "UID=#{ids[0]}", '--build-arg', "GID=#{ids[1]}"] if name == 'base' && ids
    ['build', '--no-cache', *args, '-t', "#{prefix}:#{name}", File.join(@images_dir, name)]
  end
end
