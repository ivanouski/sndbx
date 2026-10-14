# frozen_string_literal: true

require_relative 'test_helper'
require 'pty'
require 'shellwords'

class SndbxTest < Minitest::Test
  include SndbxTestHelper

  REPO_IMAGES_DIR = File.expand_path('../images', __dir__)

  FAKE_DOCKER = <<~RUBY
    #!#{RbConfig.ruby}
    require 'json'

    File.write(ENV.fetch('DOCKER_LOG'), "\#{ARGV.to_json}\\n", mode: 'a')
    images_path = ENV.fetch('DOCKER_IMAGES')
    images = File.exist?(images_path) ? File.readlines(images_path, chomp: true) : []
    containers_path = ENV.fetch('DOCKER_CONTAINERS')
    containers = File.exist?(containers_path) ? File.readlines(containers_path, chomp: true).map(&:split) : []

    case ARGV
    in ['info']
      exit(!ENV.key?('DOCKER_STOPPED'))
    in ['image', 'inspect', tag]
      exit(images.include?(tag))
    in ['ps', *filters, '--format', '{{.Names}}']
      status = filters.include?('--all') ? filters.find { |arg| arg.start_with?('status=') }[7..] : 'running'
      containers.each do |image, name, state = 'running'|
        puts name if filters.include?("label=sndbx.image=\#{image}") && state == status
      end
    in ['build', *, '-t', tag, _]
      exit(false) if ENV['DOCKER_BROKEN'] == tag
      File.write(images_path, "\#{tag}\\n", mode: 'a')
    else
    end
  RUBY

  def setup
    super
    @no_bin = mkdir('no-bin')
    @docker_log = File.join(@root, 'docker.log')
    @docker_images = File.join(@root, 'docker-images')
    @docker_containers = File.join(@root, 'docker-containers')

    write_executable(File.join(@bin, 'docker'), FAKE_DOCKER)
  end

  def test_no_arguments_prints_help
    assert_help 'Usage: sndbx command [options]', sndbx
  end

  def test_help
    assert_help 'Usage: sndbx command [options]', sndbx('-h')
  end

  def test_run_help
    assert_help 'Usage: sndbx run IMAGE [options] [-- DOCKER_ARGS]', sndbx('run', '-h')
  end

  def test_build_help
    assert_help 'Usage: sndbx build IMAGE | --all', sndbx('build', '--help')
  end

  def test_exec_help
    assert_help 'Usage: sndbx exec IMAGE CONTAINER COMMAND [ARGS] | IMAGE --list', sndbx('exec', '-h')
  end

  def test_resume_help
    assert_help 'Usage: sndbx resume IMAGE CONTAINER | IMAGE --list', sndbx('resume', '-h')
  end

  def test_logs_help
    assert_help 'Usage: sndbx logs [options]', sndbx('logs', '-h')
  end

  def test_version
    result = sndbx('-V')

    assert_predicate result, :success?
    assert_match(/\Asndbx \d+\.\d+\.\d+\n\z/, result.out)
  end

  def test_unknown_command
    assert_error "unknown command 'foo', expected one of: run, build, exec, resume, logs", sndbx('foo')
  end

  def test_invalid_option
    assert_error 'invalid option: --bogus', sndbx('--bogus')
  end

  def test_first_run_creates_example_settings
    result = sndbx('run', 'claude')

    refute_predicate result, :success?
    assert_equal "Created an example config at #{settings_path}, edit it and run again.\n", result.err
    assert_equal(['claude'], YAML.safe_load_file(settings_path)['images'].map { |image| image['name'] })
  end

  def test_unknown_image
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 'ubuntu' }])

    assert_error "unknown image 'nope', expected one of: claude, codex", sndbx('run', 'nope')
  end

  def test_image_without_user
    write_settings(images: [{ name: 'claude' }])

    assert_error "image 'claude' has no user in #{settings_path}", sndbx('run', 'claude')
  end

  def test_unquoted_port
    write_settings(images: [{ name: 'claude', user: 'ubuntu', ports: [3000] }])

    assert_settings_error 'images[0].ports[0]: invalid type: integer `3000`, expected a string', sndbx('run', 'claude')
  end

  def test_docker_args_must_be_strings
    write_settings(images: [{ name: 'claude', user: 'ubuntu', docker_args: [8] }])

    assert_settings_error 'images[0].docker_args[0]: invalid type: integer `8`, expected a string',
                          sndbx('run', 'claude')
  end

  def test_invalid_tag_prefix
    write_settings(tag_prefix: 'Not Valid', images: [])

    assert_error "invalid #{settings_path}: tag_prefix must be a lowercase image name, e.g. sndbx",
                 sndbx('run', 'claude')
  end

  def test_images_must_be_a_list
    write_settings(images: 'claude')

    assert_settings_error 'images: invalid type: string "claude", expected a sequence', sndbx('run', 'claude')
  end

  def test_images_need_a_name
    write_settings(images: [{ user: 'ubuntu' }])

    assert_settings_error 'images[0]: missing field `name`', sndbx('run', 'claude')
  end

  def test_settings_must_be_a_mapping
    write_settings_file("- claude\n")

    assert_error "invalid #{settings_path}: invalid type: sequence, expected a mapping with an images list",
                 sndbx('run', 'claude')
  end

  def test_invalid_yaml
    write_settings_file("images: [\n")

    result = sndbx('run', 'claude')

    refute_predicate result, :success?
    assert_match(/\AError: invalid #{Regexp.escape(settings_path)}: /, result.err)
  end

  def test_run_mounts_the_current_directory
    write_claude_settings

    result = sndbx('run', 'claude')

    assert_predicate result, :success?
    assert_equal [*run_options('project'), *volume('project'), *config_volume, 'sndbx:claude'], docker_calls.last
    assert File.directory?(default_config_dir)
  end

  def test_run_checks_docker_before_running
    write_claude_settings

    sndbx('run', 'claude')

    assert_equal [['info'], ['image', 'inspect', 'sndbx:claude']], docker_calls.first(2)
  end

  def test_run_with_volumes_starts_in_the_last_one
    write_claude_settings
    api = mkdir('work/api')
    web = mkdir('work/web')

    sndbx('run', 'claude', '-v', api, '-v', web)

    assert_equal [*run_options('web'), *volume('api', api), *volume('web', web), *config_volume, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_with_relative_volume
    write_claude_settings
    api = mkdir('work/api')

    sndbx('run', 'claude', '-v', '../api')

    assert_equal [*run_options('api'), *volume('api', api), *config_volume, 'sndbx:claude'], docker_calls.last
  end

  def test_run_with_workdir
    write_claude_settings
    api = mkdir('work/api')
    web = mkdir('work/web')

    sndbx('run', 'claude', '-v', api, '-v', web, '-w', 'api')

    assert_includes docker_calls.last, '--workdir=/workspace/api'
  end

  def test_workdir_must_be_a_volume
    write_claude_settings

    assert_error "workdir 'api' is not a directory volume, expected one of: project",
                 sndbx('run', 'claude', '-w', 'api')
  end

  def test_run_with_volume_target_and_options_keeps_the_current_directory
    write_claude_settings
    sdk = mkdir('work/garmin/sdk')

    sndbx('run', 'claude', '-v', '../garmin/sdk:/opt/garmin/sdk:ro')

    assert_equal [*run_options('project'), *volume('project'), '-v', "#{sdk}:/opt/garmin/sdk:ro", *config_volume,
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_with_volume_target_in_the_workspace_replaces_the_current_directory
    write_claude_settings
    api = mkdir('work/api')

    sndbx('run', 'claude', '-v', "#{api}:/workspace/backend/")

    assert_equal [*run_options('backend'), *volume('backend', api), *config_volume, 'sndbx:claude'], docker_calls.last
  end

  def test_run_with_file_volume
    write_claude_settings
    gitconfig = File.join(@home, '.gitconfig')
    File.write(gitconfig, '')

    sndbx('run', 'claude', '-v', '~/.gitconfig:/home/ubuntu/.gitconfig:ro')

    assert_equal [*run_options('project'), *volume('project'), '-v', "#{gitconfig}:/home/ubuntu/.gitconfig:ro",
                  *config_volume, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_with_absolute_workdir
    write_claude_settings
    sdk = mkdir('work/sdk')

    sndbx('run', 'claude', '-v', "#{sdk}:/opt/sdk", '-w', '/opt/sdk')

    assert_includes docker_calls.last, '--workdir=/opt/sdk'
  end

  def test_workdir_must_be_a_directory_volume
    write_claude_settings
    File.write(File.join(@work, 'notes.txt'), '')

    assert_error "workdir '/notes.txt' is not a directory volume, expected one of: project",
                 sndbx('run', 'claude', '-v', 'notes.txt:/notes.txt', '-w', '/notes.txt')
  end

  def test_run_with_volume_at_the_workspace_root
    write_claude_settings
    api = mkdir('work/api')

    sndbx('run', 'claude', '-v', "#{api}:/workspace", '-v', '.:/opt/project')

    assert_equal ['run', '--rm', '-it', '--label=sndbx.image=claude', '--workdir=/workspace',
                  '-v', "#{api}:/workspace", '-v', "#{@work}:/opt/project", *config_volume, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_workdir_at_the_workspace_root
    write_claude_settings

    assert_error "workdir 'api' is not a directory volume, expected one of: /workspace",
                 sndbx('run', 'claude', '-v', '.:/workspace', '-w', 'api')
  end

  def test_run_starts_in_a_directory_volume_outside_the_workspace_without_one_inside
    write_claude_settings
    sdk = mkdir('work/sdk')
    File.write(File.join(@work, 'notes.txt'), '')

    sndbx('run', 'claude', '-v', 'notes.txt:/workspace/notes.txt', '-v', "#{sdk}:/opt/sdk")

    assert_includes docker_calls.last, '--workdir=/opt/sdk'
  end

  def test_run_needs_a_directory_volume
    write_claude_settings
    File.write(File.join(@work, 'notes.txt'), '')

    assert_error 'no directory volume to start in', sndbx('run', 'claude', '-v', 'notes.txt:/workspace/notes.txt')
  end

  def test_volume_without_target_must_be_a_directory
    write_claude_settings
    File.write(File.join(@work, 'notes.txt'), '')

    assert_error "#{File.join(@work, 'notes.txt')} is not a directory", sndbx('run', 'claude', '-v', 'notes.txt')
  end

  def test_volume_must_exist
    write_claude_settings
    missing = File.join(@root, 'missing')

    assert_error "#{missing} does not exist", sndbx('run', 'claude', '-v', "#{missing}:/missing")
  end

  def test_volume_target_must_be_absolute
    write_claude_settings

    assert_error "invalid volume '.:workspace', DST must be an absolute path",
                 sndbx('run', 'claude', '-v', '.:workspace')
  end

  def test_invalid_volume
    write_claude_settings

    assert_error "invalid volume '.:/a:ro:b', expected SRC[:DST[:OPTS]]", sndbx('run', 'claude', '-v', '.:/a:ro:b')
    assert_error "invalid volume '.::ro', expected SRC[:DST[:OPTS]]", sndbx('run', 'claude', '-v', '.::ro')
  end

  def test_volume_targets_must_be_unique
    write_claude_settings
    first = mkdir('a/project')
    second = mkdir('b/project')

    assert_error 'more than one volume is mounted at /workspace/project',
                 sndbx('run', 'claude', '-v', first, '-v', second)
  end

  def test_refuses_to_mount_home
    write_claude_settings

    assert_error "refusing to mount #{@home}", sndbx('run', 'claude', '-v', @home)
  end

  def test_refuses_to_mount_home_with_a_target
    write_claude_settings

    assert_error "refusing to mount #{@home}", sndbx('run', 'claude', '-v', '~:/home/ubuntu/host')
  end

  def test_refuses_to_mount_a_link_to_home
    write_claude_settings
    link = File.join(@root, 'link')
    File.symlink(@home, link)

    assert_error "refusing to mount #{link}", sndbx('run', 'claude', '-v', link)
  end

  def test_missing_home
    missing = File.join(@root, 'missing')

    assert_predicate sndbx('-h', env: { 'HOME' => missing }), :success?
    assert_predicate sndbx('run', '--list', env: { 'HOME' => missing }), :success?
  end

  def test_refuses_to_mount_root
    write_claude_settings

    assert_error 'refusing to mount /', sndbx('run', 'claude', '-v', '/')
  end

  def test_run_publishes_ports
    write_settings(images: [{ name: 'claude', user: 'ubuntu', ports: ['127.0.0.1:3080:3080', '3000'] }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude', '-p', '3000', '-p', '8080:80', '-p', '53/udp')

    assert_equal [*run_options('project'), *volume('project'), *config_volume,
                  '-p', '127.0.0.1:3080:3080', '-p', '3000:3000', '-p', '8080:80', '-p', '53:53/udp', 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_keeps_the_container
    write_claude_settings

    sndbx('run', 'claude', '--keep')

    assert_equal ['run', '-it', '--label=sndbx.image=claude', '--workdir=/workspace/project', *volume('project'),
                  *config_volume, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_invalid_port
    write_claude_settings

    assert_error "invalid port 'http', expected PORT or HOST:CONTAINER", sndbx('run', 'claude', '-p', 'http')
  end

  def test_port_out_of_range
    write_claude_settings

    assert_error "invalid port '8080:70000', ports go up to 65535", sndbx('run', 'claude', '-p', '8080:70000')
  end

  def test_run_uses_image_settings
    write_settings(
      tag_prefix: 'mine',
      images: [{
        name: 'claude',
        user: 'dev',
        config_dir: File.join(@root, 'claude-config'),
        config_target: '.config/claude',
        docker_args: ['--memory=8g', '--init']
      }]
    )
    add_docker_image('mine:claude')

    sndbx('run', 'claude')

    assert_equal [*run_options('project'), *volume('project'),
                  '-v', "#{File.join(@root, 'claude-config')}:/home/dev/.config/claude",
                  '--memory=8g', '--init', 'mine:claude'],
                 docker_calls.last
  end

  def test_relative_config_dir_resolves_against_the_settings_dir
    write_settings(images: [{ name: 'claude', user: 'ubuntu', config_dir: 'claude-docker' }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude')

    config_dir = File.join(@home, '.config', 'sndbx', 'claude-docker')
    assert File.directory?(config_dir)
    assert_equal [*run_options('project'), *volume('project'), '-v', "#{config_dir}:/home/ubuntu/.claude",
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_home_config_dir_resolves_against_home
    write_settings(images: [{ name: 'claude', user: 'ubuntu', config_dir: '~/.claude_docker' }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude')

    config_dir = File.join(@home, '.claude_docker')
    assert File.directory?(config_dir)
    assert_equal [*run_options('project'), *volume('project'), '-v', "#{config_dir}:/home/ubuntu/.claude",
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_config_dir_must_be_a_path
    write_settings(images: [{ name: 'claude', user: 'ubuntu', config_dir: 42 }])

    assert_settings_error 'images[0].config_dir: invalid type: integer `42`, expected a string', sndbx('run', 'claude')
  end

  def test_other_images_must_be_valid
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 5 }])

    assert_settings_error 'images[1].user: invalid type: integer `5`, expected a string', sndbx('run', 'claude')
  end

  def test_config_dir_must_not_be_empty
    write_settings(images: [{ name: 'claude', user: 'ubuntu', config_dir: '' }])

    assert_error "image 'claude' has a config_dir that isn't a path in #{settings_path}", sndbx('run', 'claude')
  end

  def test_run_passes_its_own_options_after_the_separator_to_docker
    write_claude_settings

    sndbx('run', 'claude', '--', '-v', '/tmp:/tmp', '-p', '3000', '-w', '/tmp')

    assert_equal [*run_options('project'), *volume('project'), *config_volume,
                  '-v', '/tmp:/tmp', '-p', '3000', '-w', '/tmp', 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_passes_docker_args_after_the_image_ones
    write_settings(images: [{ name: 'claude', user: 'ubuntu', docker_args: ['--memory=8g'] }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude', '-v', '.', '--', '--memory=16g', '-e', 'FOO=bar')

    assert_equal [*run_options('project'), *volume('project'), *config_volume,
                  '--memory=8g', '--memory=16g', '-e', 'FOO=bar', 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_takes_the_image_before_docker_args
    write_claude_settings

    assert_error 'expected exactly one image', sndbx('run', '--', 'claude')
  end

  def test_run_uses_image_tag
    write_settings(images: [{ name: 'claude', user: 'ubuntu', tag: 'my-claude:latest' }])
    add_docker_image('my-claude:latest')

    sndbx('run', 'claude')

    assert_equal 'my-claude:latest', docker_calls.last.last
  end

  def test_run_without_docker
    write_claude_settings

    assert_error 'docker is not installed', sndbx('run', 'claude', docker: false)
  end

  def test_run_with_docker_stopped
    write_claude_settings

    assert_error 'docker is not running', sndbx('run', 'claude', env: { 'DOCKER_STOPPED' => '1' })
  end

  def test_run_with_missing_image
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])

    assert_error "docker image 'sndbx:claude' does not exist", sndbx('run', 'claude')
  end

  def test_run_needs_one_image
    assert_error 'expected exactly one image', sndbx('run')
    assert_error 'expected exactly one image', sndbx('run', 'claude', 'codex')
  end

  def test_run_list
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 'ubuntu' }])

    assert_equal "claude\ncodex\n", sndbx('run', '--list').out
  end

  def test_run_list_without_settings_does_not_create_them
    result = sndbx('run', '--list')

    assert_predicate result, :success?
    assert_empty result.out
    refute File.exist?(settings_path)
  end

  def test_build_list
    write_build_settings
    mkdir('images/not-an-image')

    assert_equal "base\nclaude\nplain\n", sndbx('build', '--list').out
  end

  def test_build_list_defaults_to_the_repo_images
    result = sndbx('build', '--list')

    assert_equal Dir.glob('*/Dockerfile', base: REPO_IMAGES_DIR).map { |path| File.dirname(path) }.sort,
                 result.out.lines(chomp: true)
    refute File.exist?(settings_path)
  end

  def test_build_with_relative_images_dir
    write_build_settings(images_dir: '../../images', home: @root)

    assert_equal "base\nclaude\nplain\n", sndbx('build', '--list', env: { 'HOME' => @root }).out
  end

  def test_build_builds_missing_base_first
    write_build_settings

    result = sndbx('build', 'claude')

    assert_predicate result, :success?
    assert_equal [build_call('base'), build_call('claude', base: true)], docker_builds
  end

  def test_build_keeps_existing_base
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', 'claude')

    assert_equal [build_call('claude', base: true)], docker_builds
  end

  def test_build_image_without_base
    write_build_settings

    sndbx('build', 'plain')

    assert_equal [build_call('plain')], docker_builds
  end

  def test_build_base
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', 'base')

    assert_equal [build_call('base')], docker_builds
  end

  def test_build_all_builds_base_first
    write_build_settings
    write_dockerfile('alpine', "FROM alpine\n")
    add_docker_image('sndbx:base')

    sndbx('build', '--all')

    assert_equal [build_call('base'), build_call('alpine'), build_call('claude', base: true), build_call('plain')],
                 docker_builds
  end

  def test_build_uses_tag_prefix
    write_build_settings(tag_prefix: 'mine')

    sndbx('build', 'claude')

    assert_equal [build_call('base', prefix: 'mine'), build_call('claude', base: true, prefix: 'mine')], docker_builds
  end

  def test_build_unknown_image
    write_build_settings

    assert_error "unknown image 'nope', expected one of: base, claude, plain", sndbx('build', 'nope')
  end

  def test_build_failure
    write_build_settings

    result = sndbx('build', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx:base' })

    assert_error 'failed to build sndbx:base', result
    assert_equal [build_call('base')], docker_builds
  end

  def test_build_without_docker
    write_build_settings

    assert_error 'docker is not installed', sndbx('build', 'base', docker: false)
  end

  def test_build_with_missing_images_dir
    missing = File.join(@root, 'missing')
    write_settings(images_dir: missing, images: [])

    assert_error "images directory #{missing} does not exist, set images_dir in #{settings_path}",
                 sndbx('build', 'base')
  end

  def test_build_needs_one_image_or_all
    write_build_settings

    assert_error 'expected exactly one image, or --all', sndbx('build')
    assert_error 'expected exactly one image, or --all', sndbx('build', 'base', 'plain')
    assert_error 'expected exactly one image, or --all', sndbx('build', '--all', 'base')
  end

  def test_exec
    write_exec_settings

    sndbx('exec', 'claude', 'gifted_mcnulty', 'bash')

    assert_equal [['info'], ['ps', '--filter', 'label=sndbx.image=claude', '--format', '{{.Names}}'],
                  ['exec', '-i', 'gifted_mcnulty', 'bash']],
                 docker_calls
  end

  def test_exec_as_user
    write_exec_settings

    sndbx('exec', '-u', 'root', 'claude', 'gifted_mcnulty', 'bash')
    sndbx('exec', 'claude', '--user=root', 'gifted_mcnulty', 'bash')
    sndbx('exec', 'claude', 'gifted_mcnulty', '-u', 'root', 'bash')

    assert_equal [['exec', '-i', '--user=root', 'gifted_mcnulty', 'bash']] * 3, docker_execs
  end

  def test_exec_passes_options_after_the_command_to_it
    write_exec_settings

    sndbx('exec', 'claude', 'gifted_mcnulty', 'ls', '-la', '-u', 'x', '--help', '--')

    assert_equal [['exec', '-i', 'gifted_mcnulty', 'ls', '-la', '-u', 'x', '--help', '--']], docker_execs
  end

  def test_exec_with_a_terminal
    write_exec_settings

    PTY.spawn({ 'HOME' => @home, 'PATH' => @bin, 'DOCKER_LOG' => @docker_log, 'DOCKER_IMAGES' => @docker_images,
                'DOCKER_CONTAINERS' => @docker_containers },
              *COMMAND, 'exec', 'claude', 'gifted_mcnulty', 'bash', chdir: @work) do |out, _in, pid|
      begin
        out.read
      rescue Errno::EIO
        # the pty closes when sndbx exits
      end
      Process.wait(pid)
    end

    assert_equal [['exec', '-it', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_needs_a_nonempty_user
    write_exec_settings

    assert_error 'expected a user', sndbx('exec', '-u', '', 'claude', 'gifted_mcnulty', 'bash')
    assert_error 'expected a user', sndbx('exec', '--user=', 'claude', 'gifted_mcnulty', 'bash')
    refute File.exist?(@docker_log)
  end

  def test_exec_after_double_dash
    write_exec_settings

    sndbx('exec', 'claude', 'gifted_mcnulty', '--', '-weird')
    sndbx('exec', '--', 'claude', 'gifted_mcnulty', 'bash')

    assert_equal [['exec', '-i', 'gifted_mcnulty', '-weird'], ['exec', '-i', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_needs_an_image_a_container_and_a_command
    write_exec_settings

    assert_error 'expected an image, a container and a command', sndbx('exec')
    assert_error 'expected an image, a container and a command', sndbx('exec', 'claude')
    assert_error 'expected an image, a container and a command', sndbx('exec', 'claude', 'gifted_mcnulty')
  end

  def test_exec_unknown_image
    write_exec_settings

    assert_error "unknown image 'nope', expected one of: claude, codex", sndbx('exec', 'nope', 'gifted_mcnulty', 'bash')
  end

  def test_exec_container_of_another_image
    write_exec_settings

    assert_error "container 'eager_turing' is not a running claude sandbox, " \
                 'expected one of: gifted_mcnulty, quirky_hopper',
                 sndbx('exec', 'claude', 'eager_turing', 'bash')
    assert_empty docker_execs
  end

  def test_exec_without_running_containers
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])

    assert_error 'no running claude sandbox', sndbx('exec', 'claude', 'gifted_mcnulty', 'bash')
  end

  def test_exec_without_docker
    write_exec_settings

    assert_error 'docker is not installed', sndbx('exec', 'claude', 'gifted_mcnulty', 'bash', docker: false)
  end

  def test_exec_list
    write_exec_settings

    assert_equal "gifted_mcnulty\nquirky_hopper\n", sndbx('exec', 'claude', '--list').out
    assert_equal "eager_turing\n", sndbx('exec', '-l', 'codex').out
    assert_empty sndbx('exec', 'opencode', '--list').out
  end

  def test_exec_list_without_settings_does_not_create_them
    add_docker_container('claude', 'gifted_mcnulty')

    assert_equal "gifted_mcnulty\n", sndbx('exec', 'claude', '--list').out
    refute File.exist?(settings_path)
  end

  def test_exec_list_needs_one_image
    assert_error 'expected exactly one image', sndbx('exec', '--list')
    assert_error 'expected exactly one image', sndbx('exec', 'claude', 'gifted_mcnulty', '--list')
  end

  def test_resume
    write_resume_settings

    sndbx('resume', 'claude', 'sleepy_babbage')

    assert_equal [['info'],
                  ['ps', '--all', '--filter', 'status=exited', '--filter', 'label=sndbx.image=claude',
                   '--format', '{{.Names}}'],
                  ['start', '-ai', 'sleepy_babbage']],
                 docker_calls
  end

  def test_resume_running_container
    write_resume_settings

    assert_error "container 'gifted_mcnulty' is not a stopped claude sandbox, expected one of: sleepy_babbage",
                 sndbx('resume', 'claude', 'gifted_mcnulty')
    assert_empty docker_starts
  end

  def test_resume_without_stopped_containers
    write_exec_settings

    assert_error 'no stopped claude sandbox', sndbx('resume', 'claude', 'gifted_mcnulty')
  end

  def test_resume_unknown_image
    write_resume_settings

    assert_error "unknown image 'nope', expected one of: claude, codex", sndbx('resume', 'nope', 'sleepy_babbage')
  end

  def test_resume_needs_an_image_and_a_container
    write_resume_settings

    assert_error 'expected an image and a container', sndbx('resume')
    assert_error 'expected an image and a container', sndbx('resume', 'claude')
    assert_error 'expected an image and a container', sndbx('resume', 'claude', 'sleepy_babbage', 'bash')
  end

  def test_resume_without_docker
    write_resume_settings

    assert_error 'docker is not installed', sndbx('resume', 'claude', 'sleepy_babbage', docker: false)
  end

  def test_resume_list
    write_resume_settings

    assert_equal "sleepy_babbage\n", sndbx('resume', 'claude', '--list').out
    assert_empty sndbx('resume', '-l', 'codex').out
  end

  def test_resume_list_needs_one_image
    assert_error 'expected exactly one image', sndbx('resume', '--list')
    assert_error 'expected exactly one image', sndbx('resume', 'claude', 'sleepy_babbage', '--list')
  end

  def test_logs_docker_commands
    write_resume_settings
    add_docker_image('sndbx:claude')
    dir = mkdir('work/project/my dir')

    sndbx('run', 'claude', '-v', dir)
    sndbx('exec', 'claude', 'gifted_mcnulty', 'bash')
    sndbx('resume', 'claude', 'sleepy_babbage')

    assert_equal [['docker', *run_options('my dir'), '-v', "#{dir}:/workspace/my dir", *config_volume, 'sndbx:claude'],
                  %w[docker exec -i gifted_mcnulty bash],
                  %w[docker start -ai sleepy_babbage]],
                 logged_commands
    assert(log_lines.all? { |line| line.match?(/\A\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d docker /) })
  end

  def test_logs_builds
    write_build_settings

    sndbx('build', 'plain')

    assert_equal [['docker', 'build', '--no-cache', '-t', 'sndbx:plain', File.join(@images_dir, 'plain')]],
                 logged_commands
  end

  def test_does_not_log_lists
    write_resume_settings

    sndbx('run', '--list')
    sndbx('exec', 'claude', '--list')
    sndbx('resume', 'claude', '--list')

    refute File.exist?(log_path)
  end

  def test_runs_when_the_log_cannot_be_written
    write_claude_settings
    File.write(mkdir('home/.local').then { |dir| File.join(dir, 'state') }, '')

    result = sndbx('run', 'claude')

    assert_predicate result, :success?
    assert_match(/\AWarning: can't write #{Regexp.escape(log_path)}: /, result.err)
    assert_equal run_options('project'), docker_calls.last.first(5)
  end

  def test_logs_follows_the_log
    write_fake_tail
    FileUtils.mkdir_p(File.dirname(log_path))
    File.write(log_path, '')

    result = sndbx('logs')

    assert_predicate result, :success?
    assert_equal ['-n', '10', '-f', log_path], JSON.parse(result.out)
  end

  def test_logs_with_lines
    write_fake_tail
    FileUtils.mkdir_p(File.dirname(log_path))
    File.write(log_path, '')

    assert_equal ['-n', '50', '-f', log_path], JSON.parse(sndbx('logs', '-n', '50').out)
    assert_equal ['-n', '0', '-f', log_path], JSON.parse(sndbx('logs', '--lines=0').out)
  end

  def test_logs_without_a_log
    write_fake_tail

    assert_error "no log yet at #{log_path}", sndbx('logs')
  end

  def test_logs_without_tail
    FileUtils.mkdir_p(File.dirname(log_path))
    File.write(log_path, '')

    assert_error 'tail is not installed', sndbx('logs')
  end

  def test_logs_needs_a_line_count
    assert_error 'invalid argument: -n x', sndbx('logs', '-n', 'x')
    assert_error 'expected a line count of 0 or more', sndbx('logs', '-n', '-1')
  end

  def test_logs_takes_no_arguments
    assert_error 'expected no arguments', sndbx('logs', 'claude')
  end

  private

  def write_fake_tail
    write_executable(File.join(@bin, 'tail'), "#!#{RbConfig.ruby}\nrequire 'json'\nputs ARGV.to_json\n")
  end

  def log_path
    File.join(@home, '.local', 'state', 'sndbx', 'sndbx.log')
  end

  def log_lines
    File.readlines(log_path, chomp: true)
  end

  def logged_commands
    log_lines.map { |line| Shellwords.split(line.split(' ', 2).last) }
  end

  def sndbx(*args, docker: true, env: {})
    super(*args, env: {
      'PATH' => docker ? @bin : @no_bin,
      'DOCKER_LOG' => @docker_log,
      'DOCKER_IMAGES' => @docker_images,
      'DOCKER_CONTAINERS' => @docker_containers
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

  def assert_help(usage, result)
    assert_predicate result, :success?
    assert_equal "#{usage}\n", result.out.lines.first
    refute File.exist?(settings_path)
    refute File.exist?(@docker_log)
  end

  def write_claude_settings
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])
    add_docker_image('sndbx:claude')
  end

  def write_build_settings(images_dir: @images_dir, **settings)
    write_dockerfile('base', "FROM ubuntu\n")
    write_dockerfile('claude', "ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n")
    write_dockerfile('plain', "FROM ubuntu\n")
    write_settings(images_dir: images_dir, images: [], **settings)
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

  def add_docker_container(image, name, state = 'running')
    File.write(@docker_containers, "#{image} #{name} #{state}\n", mode: 'a')
  end

  def add_docker_image(tag)
    File.write(@docker_images, "#{tag}\n", mode: 'a')
  end

  def docker_calls
    File.readlines(@docker_log).map { |line| JSON.parse(line) }
  end

  def docker_execs
    docker_calls.select { |call| call.first == 'exec' }
  end

  def docker_starts
    docker_calls.select { |call| call.first == 'start' }
  end

  def docker_builds
    docker_calls.select { |call| call.first == 'build' }
  end

  def run_options(workdir)
    ['run', '--rm', '-it', '--label=sndbx.image=claude', "--workdir=/workspace/#{workdir}"]
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

  def build_call(name, base: false, prefix: 'sndbx')
    args = base ? ['--build-arg', "TAG_PREFIX=#{prefix}"] : []
    ['build', '--no-cache', *args, '-t', "#{prefix}:#{name}", File.join(@images_dir, name)]
  end
end
