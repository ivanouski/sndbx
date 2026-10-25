# frozen_string_literal: true

require_relative 'sndbx_test_case'

class BuildTest < SndbxTestCase
  def test_build_with_relative_images_dir
    write_build_settings(images_dir: '../../images', home: @root)

    assert_equal "claude\nplain\n", sndbx('list', 'buildable', env: { 'HOME' => @root }).out
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

  def test_build_base_keeps_the_image_user_with_a_daemon_that_maps_users
    write_build_settings

    %w[rootless userns].each do |option|
      options = "[name=seccomp,profile=builtin name=#{option}]"
      result = sndbx('build', '-b', env: { 'DOCKER_SECURITY_OPTIONS' => options })

      assert_predicate result, :success?
      assert_equal build_call('base', ids: nil), docker_builds.last, option
    end
  end

  def test_build_image_without_base
    write_build_settings

    sndbx('build', 'plain')

    assert_equal [build_call('plain')], docker_builds
  end

  def test_build_images_in_order
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', 'plain', 'claude', 'plain')

    assert_equal [build_call('plain'), build_call('claude', base: true)], docker_builds
  end

  def test_build_builds_missing_base_once
    write_build_settings
    write_dockerfile('codex', "ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n")

    sndbx('build', 'plain', 'claude', 'codex')

    assert_equal [build_call('base'), build_call('plain'), build_call('claude', base: true),
                  build_call('codex', base: true)],
                 docker_builds
  end

  def test_build_rebuilds_base_first
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', '--rebuild-base', 'plain', 'claude')

    assert_equal [build_call('base'), build_call('plain'), build_call('claude', base: true)], docker_builds
  end

  def test_build_rebuilds_only_base
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', '-b')

    assert_equal [build_call('base')], docker_builds
  end

  def test_build_without_base_dockerfile
    write_build_settings
    File.delete(File.join(@images_dir, 'base', 'Dockerfile'))

    assert_error "can't read #{File.join(@images_dir, 'base', 'Dockerfile')}: No such file or directory (os error 2)",
                 sndbx('build', '-b')
  end

  def test_build_rejects_base
    write_build_settings

    assert_error 'base is built with --rebuild-base', sndbx('build', 'claude', 'base')
    refute File.exist?(@docker_log)
  end

  def test_build_uses_tag_prefix
    write_build_settings(tag_prefix: 'mine')

    sndbx('build', 'claude')

    assert_equal [build_call('base', prefix: 'mine'), build_call('claude', base: true, prefix: 'mine')], docker_builds
  end

  def test_build_unknown_image
    write_build_settings

    assert_error "unknown image 'nope', expected one of: claude, plain", sndbx('build', 'plain', 'nope')
    refute File.exist?(@docker_log)
  end

  def test_build_failure
    write_build_settings

    result = sndbx('build', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx:base' })

    assert_error 'failed to build sndbx:base', result
    assert_equal [build_call('base')], docker_builds
  end

  def test_build_stops_at_the_first_failure
    write_build_settings
    add_docker_image('sndbx:base')

    result = sndbx('build', 'plain', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx:plain' })

    assert_error 'failed to build sndbx:plain', result
    assert_equal [build_call('plain')], docker_builds
  end

  def test_build_without_docker
    write_build_settings

    assert_error 'docker is not installed', sndbx('build', 'plain', docker: false)
  end

  def test_build_with_missing_images_dir
    missing = File.join(@root, 'missing')
    write_settings(images_dir: missing, images: [])

    assert_error "images directory #{missing} does not exist, set images_dir in #{settings_path}",
                 sndbx('build', 'plain')
  end

  def test_build_adds_the_image_to_the_settings
    write_build_settings

    result = sndbx('build', 'plain')

    assert_predicate result, :success?
    assert_includes result.out, "==> Added plain to #{settings_path}, " \
                                "set config_target there or in #{File.join(@images_dir, 'plain', 'sndbx.yml')} " \
                                "if plain doesn't keep its config in ~/.plain\n"
    assert_equal "plain\n", sndbx('list', 'configured').out
  end

  def test_build_adds_the_keys_from_the_entry_file
    content = "images_dir: #{@images_dir}\nimages:\n  - name: codex\n"
    write_build_settings
    write_settings_file(content)
    File.write(File.join(@images_dir, 'plain', 'sndbx.yml'),
               "config_target: .config/plain\nports: ['127.0.0.1:3080:3080'] # mine\n")

    result = sndbx('build', 'plain')

    assert_includes result.out, "==> Added plain to #{settings_path}\n"
    assert_equal "#{content}  - name: 'plain'\n    config_target: .config/plain\n    " \
                 "ports: ['127.0.0.1:3080:3080'] # mine\n",
                 File.read(settings_path)
  end

  def test_build_checks_the_entry_files_first
    write_build_settings
    path = File.join(@images_dir, 'claude', 'sndbx.yml')
    File.write(path, "name: other\n")

    assert_error "invalid #{path}: name is the image directory's", sndbx('build', 'plain', 'claude')
    refute File.exist?(@docker_log)
  end

  def test_build_prints_the_entry_with_its_keys_when_it_cannot_add_it
    content = "images_dir: #{@images_dir}\nimages: []\n"
    write_build_settings
    write_settings_file(content)
    File.write(File.join(@images_dir, 'plain', 'sndbx.yml'), "proxy: false\n")

    result = sndbx('build', 'plain')

    assert result.err.end_with?(":\n  - name: 'plain'\n    proxy: false\n"), result.err
  end

  def test_build_adds_each_image
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', 'plain', 'claude')

    assert_equal %w[plain claude], configured_images
  end

  def test_build_appends_to_an_indented_list
    content = "images_dir: #{@images_dir} # mine\nimages:\n  - name: codex\n    user: dev\n"
    write_build_settings
    write_settings_file(content)

    sndbx('build', 'plain')

    assert_equal "#{content}  - name: 'plain'\n", File.read(settings_path)
  end

  def test_build_appends_to_a_list_that_is_not_indented
    write_build_settings(images: [{ name: 'codex' }])

    sndbx('build', 'plain')

    assert File.read(settings_path).end_with?("\n- name: 'plain'\n")
    assert_equal %w[codex plain], configured_images
  end

  def test_build_appends_after_the_last_line_without_a_newline
    content = "images_dir: #{@images_dir}\nimages:\n  - name: codex"
    write_build_settings
    write_settings_file(content)

    sndbx('build', 'plain')

    assert_equal "#{content}\n  - name: 'plain'\n", File.read(settings_path)
  end

  def test_build_quotes_the_name
    write_build_settings
    write_dockerfile('true', "FROM ubuntu\n")

    sndbx('build', 'true')

    assert_equal ['true'], configured_images
  end

  def test_build_keeps_a_configured_image
    write_build_settings(images: [{ name: 'plain' }, { name: 'mine', tag: 'sndbx:claude' }])
    add_docker_image('sndbx:base')
    content = File.read(settings_path)

    result = sndbx('build', 'plain', 'claude')

    assert_predicate result, :success?
    refute_includes result.out, 'Added'
    assert_equal content, File.read(settings_path)
  end

  def test_build_tells_when_the_image_entry_runs_another_tag
    write_build_settings(images: [{ name: 'plain', tag: 'my-plain:latest' }])
    content = File.read(settings_path)

    result = sndbx('build', 'plain')

    assert_includes result.out, "==> plain in #{settings_path} runs my-plain:latest, not sndbx:plain\n"
    assert_equal content, File.read(settings_path)
  end

  def test_build_appends_to_a_list_indented_by_four
    content = "images_dir: #{@images_dir}\nimages:\n    - name: codex\n      docker_args:\n        - --init\n"
    write_build_settings
    write_settings_file(content)

    sndbx('build', 'plain')

    assert_equal "#{content}    - name: 'plain'\n", File.read(settings_path)
  end

  def test_build_does_not_add_an_image_that_failed
    write_build_settings
    add_docker_image('sndbx:base')

    sndbx('build', 'plain', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx:claude' })

    assert_equal %w[plain], configured_images
  end

  def test_build_creates_the_example_settings_with_the_image
    add_docker_image('sndbx:base')

    result = sndbx('build', 'codex')

    assert_predicate result, :success?
    assert_includes result.out, "==> Created #{settings_path} with codex, "
    assert_equal %w[claude codex], configured_images
    assert_equal ['--security-opt=no-new-privileges', '--pids-limit=4096'],
                 YAML.safe_load_file(settings_path)['docker_args']
  end

  def test_build_creates_the_example_settings_that_have_the_image
    add_docker_image('sndbx:base')

    sndbx('build', 'claude')

    assert_equal %w[claude], configured_images
  end

  def test_build_warns_when_it_cannot_tell_where_to_add_the_image
    content = "images_dir: #{@images_dir}\nimages: []\n"
    write_build_settings
    write_settings_file(content)

    result = sndbx('build', 'plain')

    assert_predicate result, :success?
    assert_equal "Warning: can't add plain to the settings: " \
                 "can't tell where the images list in #{settings_path} ends.\n" \
                 "Add it to the images in #{settings_path}:\n  - name: 'plain'\n",
                 result.err
    assert_equal content, File.read(settings_path)
  end

  def test_build_warns_when_it_cannot_write_the_settings
    skip 'root writes read-only files' if Process.uid.zero?
    write_build_settings
    File.chmod(0o444, settings_path)

    result = sndbx('build', 'plain')

    assert_predicate result, :success?
    assert result.err.start_with?("Warning: can't add plain to the settings: can't write #{settings_path}: "),
           result.err
  end

  def test_build_writes_through_a_symlinked_settings_file
    write_build_settings
    target = File.join(@root, 'dotfiles-settings.yml')
    File.rename(settings_path, target)
    File.symlink(target, settings_path)

    sndbx('build', 'plain')

    assert File.symlink?(settings_path)
    assert_equal %w[plain], configured_images
  end

  def test_build_needs_an_image_or_rebuild_base
    write_build_settings

    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>...", sndbx('build')
  end
end
