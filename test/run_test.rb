# frozen_string_literal: true

require_relative 'sndbx_test_case'

class RunTest < SndbxTestCase
  def test_run_mounts_the_current_directory
    write_claude_settings

    result = sndbx('run', 'claude')

    assert_predicate result, :success?
    assert_equal [*run_options('project'), *volume('project'), *config_volume, *TERMINAL_ENV, 'sndbx:claude'],
                 docker_calls.last
    assert File.directory?(default_config_dir)
  end

  def test_run_checks_docker_before_running
    write_claude_settings

    sndbx('run', 'claude')

    assert_equal [['info'], %w[image inspect sndbx:claude],
                  ['image', 'inspect', '--format', '{{.Config.User}}', 'sndbx:claude']],
                 docker_calls.first(3)
    assert_empty docker_builds
  end

  def test_run_mounts_the_config_in_the_home_of_the_user_the_image_runs_as
    write_settings(images: [{ name: 'claude' }])
    add_docker_image('sndbx:claude', user: 'dev:staff')

    sndbx('run', 'claude')

    assert_equal ['-v', "#{default_config_dir}:/home/dev/.claude"], docker_calls.last[-6..-5]
  end

  def test_run_mounts_the_config_in_root_home_for_an_image_that_runs_as_root
    write_settings(images: [{ name: 'claude' }])
    add_docker_image('sndbx:claude', user: '')

    sndbx('run', 'claude')

    assert_equal ['-v', "#{default_config_dir}:/root/.claude"], docker_calls.last[-6..-5]
  end

  def test_run_prefers_the_user_in_the_settings
    write_settings(images: [{ name: 'claude', user: 'dev' }])
    add_docker_image('sndbx:claude', user: 'ubuntu')

    sndbx('run', 'claude')

    assert_equal ['-v', "#{default_config_dir}:/home/dev/.claude"], docker_calls.last[-6..-5]
  end

  def test_run_with_an_image_that_runs_as_a_uid
    write_settings(images: [{ name: 'claude' }])
    add_docker_image('sndbx:claude', user: '1000:1000')

    assert_error "can't tell the home of user '1000:1000' that image 'claude' runs as, " \
                 "set user to its name in #{settings_path}",
                 sndbx('run', 'claude')
    assert_empty docker_runs
  end

  def test_run_with_volumes_starts_in_the_last_one
    write_claude_settings
    api = mkdir('work/api')
    web = mkdir('work/web')

    sndbx('run', 'claude', '-v', api, '-v', web)

    assert_equal [*run_options('web'), *volume('api', api), *volume('web', web), *config_volume, *TERMINAL_ENV,
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_with_relative_volume
    write_claude_settings
    api = mkdir('work/api')

    sndbx('run', 'claude', '-v', '../api')

    assert_equal [*run_options('api'), *volume('api', api), *config_volume, *TERMINAL_ENV, 'sndbx:claude'],
                 docker_calls.last
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
                  *TERMINAL_ENV, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_with_volume_target_in_the_workspace_replaces_the_current_directory
    write_claude_settings
    api = mkdir('work/api')

    sndbx('run', 'claude', '-v', "#{api}:/workspace/backend/")

    assert_equal [*run_options('backend'), *volume('backend', api), *config_volume, *TERMINAL_ENV, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_with_file_volume
    write_claude_settings
    gitconfig = File.join(@home, '.gitconfig')
    File.write(gitconfig, '')

    sndbx('run', 'claude', '-v', '~/.gitconfig:/home/ubuntu/.gitconfig:ro')

    assert_equal [*run_options('project'), *volume('project'), '-v', "#{gitconfig}:/home/ubuntu/.gitconfig:ro",
                  *config_volume, *TERMINAL_ENV, 'sndbx:claude'],
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
                  '-v', "#{api}:/workspace", '-v', "#{@work}:/opt/project", *config_volume, *TERMINAL_ENV,
                  'sndbx:claude'],
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

  def test_refuses_to_mount_root
    write_claude_settings

    assert_error 'refusing to mount /', sndbx('run', 'claude', '-v', '/')
  end

  def test_run_publishes_ports
    write_settings(images: [{ name: 'claude', user: 'ubuntu', ports: ['127.0.0.1:3080:3080', '3000'] }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude', '-p', '3000', '-p', '8080:80', '-p', '53/udp')

    assert_equal [*run_options('project'), *volume('project'), *config_volume,
                  '-p', '127.0.0.1:3080:3080', '-p', '3000:3000', '-p', '8080:80', '-p', '53:53/udp', *TERMINAL_ENV,
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_keeps_the_container
    write_claude_settings

    sndbx('run', 'claude', '--keep')

    assert_equal ['run', '-it', '--label=sndbx.image=claude', '--workdir=/workspace/project', *volume('project'),
                  *config_volume, *TERMINAL_ENV, 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_option_spellings
    write_claude_settings
    mkdir('work/project/-dir')

    sndbx('run', '--kee', '-kp3000', '--publish=8080:80', '--pub', '53/udp', 'claude')
    sndbx('run', 'claude', '-v', '-dir', '-w', 'nope', '--workdir', '-dir')

    assert_equal [['run', '-it', '--label=sndbx.image=claude', '--workdir=/workspace/project', *volume('project'),
                   *config_volume, '-p', '3000:3000', '-p', '8080:80', '-p', '53:53/udp', *TERMINAL_ENV,
                   'sndbx:claude'],
                  [*run_options('-dir'), *volume('-dir', File.join(@work, '-dir')), *config_volume, *TERMINAL_ENV,
                   'sndbx:claude']],
                 docker_runs
  end

  def test_invalid_port
    write_claude_settings

    assert_error "invalid port 'http', expected PORT or HOST:CONTAINER", sndbx('run', 'claude', '-p', 'http')
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
                  *TERMINAL_ENV, '--memory=8g', '--init', 'mine:claude'],
                 docker_calls.last
  end

  def test_run_passes_its_own_options_after_the_separator_to_docker
    write_claude_settings

    sndbx('run', 'claude', '--', '-v', '/tmp:/tmp', '-p', '3000', '-w', '/tmp')

    assert_equal [*run_options('project'), *volume('project'), *config_volume, *TERMINAL_ENV,
                  '-v', '/tmp:/tmp', '-p', '3000', '-w', '/tmp', 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_passes_docker_args_after_the_image_ones
    write_settings(images: [{ name: 'claude', user: 'ubuntu', docker_args: ['--memory=8g'] }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude', '-v', '.', '--', '--memory=16g', '-e', 'FOO=bar')

    assert_equal [*run_options('project'), *volume('project'), *config_volume, *TERMINAL_ENV,
                  '--memory=8g', '--memory=16g', '-e', 'FOO=bar', 'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_passes_top_level_docker_args_before_the_image_ones
    write_settings(
      docker_args: ['--security-opt=no-new-privileges', '--pids-limit=4096'],
      images: [{ name: 'claude', user: 'ubuntu', docker_args: ['--pids-limit=8192'] }]
    )
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude', '--', '--memory=16g')

    assert_equal [*run_options('project'), *volume('project'), *config_volume, *TERMINAL_ENV,
                  '--security-opt=no-new-privileges', '--pids-limit=4096', '--pids-limit=8192', '--memory=16g',
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_passes_top_level_docker_args_to_images_without_their_own
    write_settings(docker_args: '--pids-limit=4096', images: [{ name: 'claude', user: 'ubuntu' }])
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude')

    assert_equal [*run_options('project'), *volume('project'), *config_volume, *TERMINAL_ENV, '--pids-limit=4096',
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_run_takes_the_image_before_docker_args
    write_claude_settings

    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>", sndbx('run', '--', 'claude')
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

  def test_run_builds_a_missing_image
    write_build_settings(images: [{ name: 'claude', user: 'ubuntu' }])
    settings = File.read(settings_path)

    result = sndbx('run', 'claude')

    assert_predicate result, :success?
    assert_equal "==> sndbx:claude doesn't exist yet, building it\n", result.out.lines.first
    assert_equal [build_call('base'), build_call('claude', base: true)], docker_builds
    assert_equal 'sndbx:claude', docker_runs.last.last
    assert_equal settings, File.read(settings_path)
  end

  def test_run_builds_the_image_its_tag_names
    write_build_settings(images: [{ name: 'foo', tag: 'sndbx:plain', user: 'ubuntu' }])

    assert_predicate sndbx('run', 'foo'), :success?
    assert_equal [build_call('plain')], docker_builds
    assert_equal 'sndbx:plain', docker_runs.last.last
  end

  def test_run_checks_the_options_before_building
    write_build_settings(images: [{ name: 'plain' }])
    missing = File.join(@root, 'missing')

    assert_error "#{missing} does not exist", sndbx('run', 'plain', '-v', missing)
    refute File.exist?(@docker_log)
  end

  def test_run_checks_the_workdir_before_building
    write_build_settings(images: [{ name: 'plain' }])

    assert_error "workdir 'nope' is not a directory volume, expected one of: project",
                 sndbx('run', 'plain', '-w', 'nope')
    refute File.exist?(@docker_log)
  end

  def test_run_stops_when_the_build_fails
    write_build_settings(images: [{ name: 'plain' }])

    assert_error 'failed to build sndbx:plain', sndbx('run', 'plain', env: { 'DOCKER_BROKEN' => 'sndbx:plain' })
    assert_empty docker_runs
  end

  def test_run_with_a_missing_image_of_another_tag
    write_build_settings(images: [{ name: 'claude', tag: 'my-claude:latest' }])

    assert_error "docker image 'my-claude:latest' that claude runs doesn't exist, and sndbx can't build it: " \
                 "it only builds sndbx:<name> from #{@images_dir}/<name>. " \
                 "Pull or build it yourself, or change claude's tag in #{settings_path}",
                 sndbx('run', 'claude')
    assert_empty docker_builds
  end

  def test_run_with_a_missing_image_without_a_dockerfile
    write_build_settings(images: [{ name: 'codex' }])

    assert_error "docker image 'sndbx:codex' that codex runs doesn't exist, and sndbx can't build it: " \
                 "there's no #{@images_dir}/codex/Dockerfile. " \
                 "Pull or build it yourself, or change codex's tag in #{settings_path}",
                 sndbx('run', 'codex')
    assert_empty docker_builds
  end

  def test_run_builds_base_for_its_tag
    write_build_settings(images: [{ name: 'foo', tag: 'sndbx:base', user: 'ubuntu' }])

    assert_predicate sndbx('run', 'foo'), :success?
    assert_equal [build_call('base')], docker_builds
    assert_equal 'sndbx:base', docker_runs.last.last
  end

  def test_run_with_a_missing_image_of_a_tag_that_names_no_image_directory
    write_build_settings
    write_dockerfile('', "FROM ubuntu\n")
    write_dockerfile('.hidden', "FROM ubuntu\n")
    write_dockerfile('plain/nested', "FROM ubuntu\n")

    ['sndbx:', 'sndbx:.hidden', 'sndbx:plain/nested'].each do |tag|
      write_build_settings(images: [{ name: 'claude', tag: tag }])

      assert_error "docker image '#{tag}' that claude runs doesn't exist, and sndbx can't build it: " \
                   "it only builds sndbx:<name> from #{@images_dir}/<name>. " \
                   "Pull or build it yourself, or change claude's tag in #{settings_path}",
                   sndbx('run', 'claude')
    end
    assert_empty docker_builds
  end

  def test_run_with_a_missing_image_and_images_dir
    missing = File.join(@root, 'missing')
    write_settings(images_dir: missing, images: [{ name: 'claude' }])

    assert_error "docker image 'sndbx:claude' that claude runs doesn't exist, and sndbx can't build it: " \
                 "images directory #{missing} does not exist, set images_dir in #{settings_path}. " \
                 "Pull or build it yourself, or change claude's tag in #{settings_path}",
                 sndbx('run', 'claude')
  end

  def test_run_needs_one_image
    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>", sndbx('run')
    assert_usage_error "unexpected argument 'codex' found", sndbx('run', 'claude', 'codex')
  end
end
