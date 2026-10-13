# frozen_string_literal: true

require_relative 'test_helper'
require 'shellwords'

class CompletionTest < Minitest::Test
  include SndbxTestHelper

  COMPLETION = File.expand_path('../completions/_sndbx', __dir__)

  COMPLETE = <<~BASH
    source "$COMPLETION"
    COMP_WORDS=(sndbx "$@")
    COMP_CWORD=$#
    _sndbx 2>/dev/null
    (( ${#COMPREPLY[@]} )) && printf '%s\\n' "${COMPREPLY[@]}"
    exit 0
  BASH

  # Speaks docker's cobra completion protocol with the output in DOCKER_COMPLETIONS,
  # fails like docker before 26 without it.
  FAKE_DOCKER = <<~RUBY
    #!#{RbConfig.ruby}
    require 'json'

    File.write(ENV.fetch('DOCKER_LOG'), "\#{ARGV.to_json}\\n", mode: 'a')
    exit if ARGV == ['info']
    if ARGV.first == 'ps'
      containers = ENV.fetch('DOCKER_CONTAINERS')
      print(File.read(containers)) if File.exist?(containers)
      exit
    end

    output = ENV.fetch('DOCKER_COMPLETIONS')
    exit(1) unless File.exist?(output)

    print(File.read(output))
  RUBY

  def setup
    super
    @docker_log = File.join(@root, 'docker.log')
    @docker_completions = File.join(@root, 'docker-completions')
    @docker_containers = File.join(@root, 'docker-containers')

    write_executable(File.join(@bin, 'sndbx'), "#!/bin/sh\nexec #{Shellwords.join(COMMAND)} \"$@\"\n")
    write_executable(File.join(@bin, 'docker'), FAKE_DOCKER)
  end

  def test_options_match_help
    [[], ['run'], ['build'], ['exec'], ['resume'], ['logs']].each do |command|
      assert_equal help_options(*command), complete(*command, '-'), "options of sndbx #{command.join(' ')}"
    end
  end

  def test_commands
    assert_equal %w[build exec logs resume run], complete('')
    assert_equal %w[resume run], complete('r')
    assert_equal %w[run], complete('ru')
  end

  def test_unknown_command
    assert_empty complete('foo', '')
  end

  def test_run_images
    write_images_settings

    assert_equal %w[claude codex], complete('run', '')
    assert_equal %w[codex], complete('run', 'co')
  end

  def test_run_images_after_option_values
    write_images_settings
    api = mkdir('work/api')

    assert_equal %w[claude codex], complete('run', '-v', api, '-w', 'api', '-p', '3000', '')
    assert_equal %w[claude codex], complete('run', '--volume', '=', api, '--publish', '=', '3000', '')
    assert_equal %w[claude codex], complete('run', '-v', api, ':', '/opt/api', ':', 'ro', '-p', '8080', ':', '80', '')
  end

  def test_volume_target_completes_nothing
    write_images_settings

    assert_empty complete('run', '-v', 'src', ':')
    assert_empty complete('run', '-v', 'src', ':', '')
    assert_empty complete('run', '-v', 'src', ':', '/opt', ':', '')
  end

  def test_docker_args_complete_with_docker
    write_docker_completions("--memory\tMemory limit\n--memory-swap\n:4\n")

    assert_equal %w[--memory --memory-swap], complete('run', 'claude', '-v', '.', '--', '--init', '--mem')
    assert_equal [['__complete', 'run', '--init', '--mem']], docker_calls
  end

  def test_docker_args_filter_candidates
    write_docker_completions("bridge\nhost\nnone\n:4\n")

    assert_equal %w[host], complete('run', 'claude', '--', '--network', 'h')
  end

  def test_docker_args_rejoin_equals_signs_and_colons
    write_docker_completions("src:/opt\nsrc:/srv\nother:/opt\n:0\n")

    assert_equal %w[/opt /srv], complete('run', 'claude', '--', '--volume', '=', 'src', ':', '/')
    assert_equal %w[:/opt :/srv], complete('run', 'claude', '--', '-v', 'src', ':')
    assert_equal [['__complete', 'run', '--volume=src:/'], ['__complete', 'run', '-v', 'src:']], docker_calls
  end

  def test_docker_args_without_candidates_complete_files
    write_docker_completions(":0\n")
    File.write(File.join(@work, 'env'), '')
    mkdir('work/project/src')

    assert_equal %w[env src], complete('run', 'claude', '--', '--env-file', '=', '')
  end

  def test_docker_args_complete_directories_when_docker_says_so
    write_docker_completions(":16\n")
    File.write(File.join(@work, 'env'), '')
    mkdir('work/project/src')

    assert_equal %w[src], complete('run', 'claude', '--', '--workdir', '')
  end

  def test_docker_args_without_file_completion
    write_docker_completions(":4\n")
    File.write(File.join(@work, 'env'), '')

    assert_empty complete('run', 'claude', '--', '-e', '')
  end

  def test_docker_args_after_a_docker_error
    write_docker_completions("--memory\n:1\n")

    assert_empty complete('run', 'claude', '--', '-')
  end

  def test_docker_args_complete_files_with_old_docker
    File.write(File.join(@work, 'env'), '')

    assert_equal %w[env], complete('run', 'claude', '--', '--env-file', '')
  end

  def test_run_no_image_after_image
    write_images_settings

    assert_empty complete('run', 'claude', '')
  end

  def test_run_images_without_settings_do_not_create_them
    assert_empty complete('run', '')
    refute File.exist?(settings_path)
  end

  def test_build_images
    write_dockerfile('base', "FROM ubuntu\n")
    write_dockerfile('claude', "FROM ubuntu\n")
    write_settings(images_dir: @images_dir, images: [])

    assert_equal %w[base claude], complete('build', '')
    assert_empty complete('build', 'base', '')
    assert_empty complete('build', '--all', '')
  end

  def test_exec_images
    write_images_settings

    assert_equal %w[claude codex], complete('exec', '')
    assert_equal %w[claude codex], complete('exec', '-u', 'root', '')
    assert_equal %w[claude codex], complete('exec', '--user', '=', 'root', '')
  end

  def test_exec_containers
    write_images_settings
    File.write(@docker_containers, "gifted_mcnulty\nquirky_hopper\n")

    assert_equal %w[gifted_mcnulty quirky_hopper], complete('exec', 'claude', '')
    assert_equal %w[quirky_hopper], complete('exec', '-u', 'root', 'claude', 'q')
    assert_equal [['info'], ['ps', '--filter', 'label=sndbx.image=claude', '--format', '{{.Names}}']],
                 docker_calls.last(2)
  end

  def test_exec_options_before_the_command
    assert_equal %w[--user], complete('exec', 'claude', 'gifted_mcnulty', '--u')
  end

  def test_exec_completes_nothing_from_the_command_on
    File.write(File.join(@work, 'README.md'), '')

    assert_empty complete('exec', 'claude', 'gifted_mcnulty', '')
    assert_empty complete('exec', 'claude', 'gifted_mcnulty', 'ls', '')
    assert_empty complete('exec', 'claude', 'gifted_mcnulty', 'ls', '-')
    assert_empty complete('exec', 'claude', 'gifted_mcnulty', '--', '-')
    assert_empty complete('exec', '-u', '')
  end

  def test_exec_list_completes_nothing_after_the_image
    write_images_settings

    assert_empty complete('exec', 'claude', '--list', '')
    assert_empty complete('exec', '-l', 'claude', '')
    assert_equal %w[claude codex], complete('exec', '--list', '')
  end

  def test_resume_images
    write_images_settings

    assert_equal %w[claude codex], complete('resume', '')
  end

  def test_resume_containers
    write_images_settings
    File.write(@docker_containers, "sleepy_babbage\nzen_lovelace\n")

    assert_equal %w[sleepy_babbage zen_lovelace], complete('resume', 'claude', '')
    assert_equal %w[zen_lovelace], complete('resume', 'claude', 'z')
    assert_equal [['info'],
                  ['ps', '--all', '--filter', 'status=exited', '--filter', 'label=sndbx.image=claude',
                   '--format', '{{.Names}}']],
                 docker_calls.last(2)
  end

  def test_resume_completes_nothing_after_the_container
    write_images_settings

    assert_empty complete('resume', 'claude', 'sleepy_babbage', '')
    assert_empty complete('resume', 'claude', '--list', '')
    assert_equal %w[claude codex], complete('resume', '--list', '')
  end

  def test_logs_completes_nothing_but_options
    write_images_settings

    assert_empty complete('logs', '')
    assert_empty complete('logs', '-n', '')
    assert_empty complete('logs', '-n', '-')
  end

  def test_volume_completes_directories
    mkdir('work/project/src')
    File.write(File.join(@work, 'README.md'), '')

    assert_equal %w[src], complete('run', '-v', '')
  end

  def test_publish_completes_nothing
    assert_empty complete('run', '-p', '')
  end

  def test_workdir_defaults_to_the_current_directory
    assert_equal %w[project], complete('run', 'claude', '-w', '')
  end

  def test_workdir_completes_volumes
    api = mkdir('work/api')
    mkdir('work/web')
    mkdir('home/notes')

    assert_equal %w[api notes web], complete('run', '-v', api, '-v', '../web', '-v', '~/notes', '-w', '')
  end

  def test_workdir_completes_volume_targets
    api = mkdir('work/api')
    mkdir('work/sdk')
    File.write(File.join(@work, 'notes.txt'), '')

    assert_equal %w[/opt/sdk project],
                 complete('run', '-v', '../sdk', ':', '/opt/sdk', ':', 'ro', '-v', 'notes.txt:/notes.txt', '-w', '')
    assert_equal %w[/opt/sdk backend],
                 complete('run', '-v', '"../sdk:/opt/sdk"', '-v', "#{api}:/workspace/backend", '-w', '')
  end

  def test_workdir_completes_the_workspace_root
    api = mkdir('work/api')

    assert_equal %w[/workspace], complete('run', '-v', api, ':', '/workspace', '-w', '')
  end

  def test_workdir_skips_missing_volumes
    api = mkdir('work/api')

    assert_equal %w[api], complete('run', '-v', api, '-v', File.join(@root, 'missing'), '-w', '')
  end

  def test_workdir_with_equals_sign
    api = mkdir('work/api')

    assert_equal %w[api], complete('run', '--volume', '=', api, '--workdir', '=', '')
    assert_equal %w[api], complete('run', '--volume', '=', api, '--workdir', '=')
  end

  private

  def complete(*words)
    env = { 'HOME' => @home, 'PATH' => "#{@bin}:#{ENV.fetch('PATH')}", 'COMPLETION' => COMPLETION,
            'DOCKER_LOG' => @docker_log, 'DOCKER_COMPLETIONS' => @docker_completions,
            'DOCKER_CONTAINERS' => @docker_containers }
    out, status = Open3.capture2(env, 'bash', '-c', COMPLETE, 'bash', *words, chdir: @work)
    assert_predicate status, :success?

    out.lines(chomp: true).sort
  end

  def help_options(*command)
    sndbx(*command, '-h').out.lines
                         .map(&:strip)
                         .select { |line| line.start_with?('-') }
                         .flat_map { |line| line.split(/\s{2,}/).first.scan(/-[-\w]+/) }
                         .sort
  end

  def write_docker_completions(output)
    File.write(@docker_completions, output)
  end

  def docker_calls
    File.readlines(@docker_log).map { |line| JSON.parse(line) }
  end

  def write_images_settings
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 'ubuntu' }])
  end
end
