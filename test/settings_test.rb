# frozen_string_literal: true

require_relative 'sndbx_test_case'

class SettingsTest < SndbxTestCase
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

  def test_images_must_be_mappings
    write_settings(images: ['claude'])

    assert_settings_error 'images[0]: invalid type: string "claude", expected a mapping with a name',
                          sndbx('run', 'claude')
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

  def test_settings_under_xdg_config_home
    config_home = mkdir('xdg-config')
    File.write(File.join(mkdir('xdg-config/sndbx'), 'settings.yml'), "images:\n  - name: codex\n    user: ubuntu\n")
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])

    assert_equal "codex\n", sndbx('list', 'configured', env: { 'XDG_CONFIG_HOME' => config_home }).out
  end

  def test_first_run_creates_example_settings_under_xdg_config_home
    config_home = File.join(@root, 'xdg-config')
    path = File.join(config_home, 'sndbx', 'settings.yml')

    result = sndbx('run', 'claude', env: { 'XDG_CONFIG_HOME' => config_home })

    assert_equal "Created an example config at #{path}, edit it and run again.\n", result.err
    assert File.exist?(path)
    refute File.exist?(settings_path)
  end

  def test_default_config_dir_under_xdg_config_home
    config_home = mkdir('xdg-config')
    File.write(File.join(mkdir('xdg-config/sndbx'), 'settings.yml'), "images:\n  - name: claude\n    user: ubuntu\n")
    add_docker_image('sndbx:claude')

    sndbx('run', 'claude', env: { 'XDG_CONFIG_HOME' => config_home })

    config_dir = File.join(config_home, 'sndbx', 'claude')
    assert File.directory?(config_dir)
    assert_includes docker_calls.last, "#{config_dir}:/home/ubuntu/.claude"
  end

  def test_relative_or_empty_xdg_config_home_is_ignored
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])
    mkdir('work/project/xdg-config/sndbx')

    assert_equal "claude\n", sndbx('list', 'configured', env: { 'XDG_CONFIG_HOME' => 'xdg-config' }).out
    assert_equal "claude\n", sndbx('list', 'configured', env: { 'XDG_CONFIG_HOME' => '' }).out
  end

  def test_top_level_docker_args_must_be_strings
    write_settings(docker_args: ['--pids-limit=4096', 8], images: [{ name: 'claude', user: 'ubuntu' }])

    assert_settings_error 'docker_args[1]: invalid type: integer `8`, expected a string', sndbx('run', 'claude')
  end

  def test_unknown_key
    write_settings(tag_prefx: 'mine', images: [])

    assert_settings_error 'unknown field `tag_prefx`, expected one of `tag_prefix`, `images_dir`, `docker_args`, ' \
                          '`proxy`, `images`',
                          sndbx('list', 'configured')
  end

  # An empty HOME is fine, Rust takes the home from the user database then
  def test_home_must_be_absolute
    write_claude_settings

    assert_error 'HOME is not set to an absolute path', sndbx('list', 'configured', env: { 'HOME' => 'relative' })
  end

  def test_unknown_image_key
    write_settings(images: [{ name: 'claude', user: 'ubuntu', docker_arg: ['--memory=8g'] }])

    assert_settings_error 'images[0]: unknown field `docker_arg`, expected one of `name`, `user`, `tag`, ' \
                          '`config_dir`, `config_target`, `ports`, `docker_args`, `proxy`',
                          sndbx('run', 'claude')
  end
end
