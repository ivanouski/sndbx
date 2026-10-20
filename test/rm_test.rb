# frozen_string_literal: true

require_relative 'sndbx_test_case'

class RmTest < SndbxTestCase
  def test_rm_removes_the_image_and_comments_out_its_entry
    write_settings_file("images:\n  - name: claude\n    ports: ['3000']\n  - name: codex\n")
    add_docker_image('sndbx:claude')

    result = sndbx('rm', 'claude')

    assert_predicate result, :success?
    assert_equal "==> Removed sndbx:claude\n==> Commented out claude in #{settings_path}\n", result.out
    assert_equal [%w[image rm sndbx:claude]], docker_image_rms
    assert_equal "images:\n  # Removed by sndbx rm on #{Date.today.iso8601}:\n  " \
                 "# - name: claude\n  #   ports: ['3000']\n  - name: codex\n",
                 File.read(settings_path)
    assert_equal "codex\n", sndbx('list', 'configured').out
  end

  def test_rm_keeps_the_config_directory
    write_claude_settings
    FileUtils.mkdir_p(default_config_dir)

    sndbx('rm', 'claude')

    assert File.directory?(default_config_dir)
  end

  def test_rm_several_images
    write_settings(images: [{ name: 'claude' }, { name: 'codex' }])
    add_docker_image('sndbx:claude')
    add_docker_image('sndbx:codex')

    sndbx('rm', 'claude', 'codex', 'claude')

    assert_equal [%w[image rm sndbx:claude], %w[image rm sndbx:codex]], docker_image_rms
    assert_nil YAML.safe_load_file(settings_path)['images']
  end

  def test_rm_the_only_entry_then_build_adds_it_again
    write_dockerfile('plain', "FROM ubuntu\n")
    write_settings_file("images_dir: #{@images_dir}\nimages:\n  - name: plain\n    user: dev\n")
    add_docker_image('sndbx:plain')

    sndbx('rm', 'plain')
    sndbx('build', 'plain')

    assert_equal [{ 'name' => 'plain' }], YAML.safe_load_file(settings_path)['images']
    assert_includes File.read(settings_path), "  # - name: plain\n  #   user: dev\n"
  end

  def test_rm_an_entry_whose_name_is_not_its_first_key
    write_settings_file("images:\n  - user: dev\n    name: claude\n  - name: codex\n")
    add_docker_image('sndbx:claude')

    sndbx('rm', 'claude')

    assert_equal "codex\n", sndbx('list', 'configured').out
  end

  def test_rm_an_image_with_its_own_tag
    write_settings(images: [{ name: 'claude', tag: 'my-claude:latest' }])
    add_docker_image('my-claude:latest')

    sndbx('rm', 'claude')

    assert_equal [%w[image rm my-claude:latest]], docker_image_rms
  end

  def test_rm_a_configured_image_that_does_not_exist
    write_settings(images: [{ name: 'claude' }])

    result = sndbx('rm', 'claude')

    assert_predicate result, :success?
    assert_equal "==> sndbx:claude doesn't exist\n==> Commented out claude in #{settings_path}\n", result.out
    assert_empty docker_image_rms
  end

  def test_rm_a_built_image_that_is_not_configured
    write_settings(images: [{ name: 'claude' }])
    add_docker_image('sndbx:plain')
    content = File.read(settings_path)

    result = sndbx('rm', 'plain')

    assert_equal "==> Removed sndbx:plain\n", result.out
    assert_equal content, File.read(settings_path)
  end

  def test_rm_without_settings
    add_docker_image('sndbx:plain')

    assert_predicate sndbx('rm', 'plain'), :success?
    refute File.exist?(settings_path)
  end

  def test_rm_unknown_image
    write_claude_settings

    assert_error "unknown image 'nope', it's neither in #{settings_path} nor built as sndbx:nope",
                 sndbx('rm', 'claude', 'nope')
    assert_empty docker_image_rms
  end

  def test_rm_refuses_base
    write_claude_settings

    assert_error "base can't be removed, the other images are built on it", sndbx('rm', 'claude', 'base')
    refute File.exist?(@docker_log)
  end

  def test_rm_refuses_images_that_containers_use
    write_settings(images: [{ name: 'plain' }, { name: 'claude' }])
    add_docker_image('sndbx:plain')
    add_docker_image('sndbx:claude')
    add_docker_container('claude', 'gifted_mcnulty')
    add_docker_container('claude', 'sleepy_babbage', 'exited')
    add_docker_container('claude', 'never_started', 'created')
    add_docker_container('-', 'by_hand', 'running', 'sndbx:claude')

    assert_error 'sndbx:claude is used by containers: gifted_mcnulty, sleepy_babbage, never_started, by_hand, ' \
                 'remove them with docker rm first',
                 sndbx('rm', 'plain', 'claude')
    assert_empty docker_image_rms
  end

  def test_rm_refuses_a_tag_another_entry_runs
    write_settings(images: [{ name: 'claude' }, { name: 'yolo', tag: 'sndbx:claude' }])
    add_docker_image('sndbx:claude')

    assert_error 'sndbx:claude is also run by yolo, rm them together', sndbx('rm', 'claude')
    assert_empty docker_image_rms
  end

  def test_rm_entries_that_share_a_tag_together
    write_settings(images: [{ name: 'claude' }, { name: 'yolo', tag: 'sndbx:claude' }, { name: 'codex' }])
    add_docker_image('sndbx:claude')

    result = sndbx('rm', 'claude', 'yolo')

    assert_equal "==> Removed sndbx:claude\n==> Commented out claude in #{settings_path}\n" \
                 "==> Commented out yolo in #{settings_path}\n",
                 result.out
    assert_equal [%w[image rm sndbx:claude]], docker_image_rms
    assert_equal "codex\n", sndbx('list', 'configured').out
  end

  def test_rm_failure
    write_claude_settings
    content = File.read(settings_path)

    assert_error 'failed to remove sndbx:claude', sndbx('rm', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx:claude' })
    assert_equal content, File.read(settings_path)
  end

  def test_rm_warns_when_it_cannot_comment_out_the_entry
    write_settings_file("images: [{name: claude}]\n")
    add_docker_image('sndbx:claude')

    result = sndbx('rm', 'claude')

    assert_predicate result, :success?
    assert_equal "Warning: can't comment out claude in the settings: " \
                 "can't tell where its entry in #{settings_path} is.\n" \
                 "Remove its entry from #{settings_path} yourself.\n",
                 result.err
    assert_equal [%w[image rm sndbx:claude]], docker_image_rms
  end

  def test_rm_writes_through_a_symlinked_settings_file
    write_claude_settings
    target = File.join(@root, 'dotfiles-settings.yml')
    File.rename(settings_path, target)
    File.symlink(target, settings_path)

    sndbx('rm', 'claude')

    assert File.symlink?(settings_path)
    assert_includes File.read(target), '# - name: claude'
  end

  def test_rm_needs_an_image
    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>...", sndbx('rm')
  end

  def test_rm_without_docker
    write_claude_settings

    assert_error 'docker is not installed', sndbx('rm', 'claude', docker: false)
  end
end
