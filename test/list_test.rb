# frozen_string_literal: true

require_relative 'sndbx_test_case'

class ListTest < SndbxTestCase
  REPO_IMAGES_DIR = File.expand_path('../images', __dir__)

  def test_list_configured
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 'ubuntu' }])

    assert_equal "claude\ncodex\n", sndbx('list', 'configured').out
  end

  def test_list_configured_without_settings_does_not_create_them
    result = sndbx('list', 'configured')

    assert_predicate result, :success?
    assert_empty result.out
    refute File.exist?(settings_path)
  end

  def test_list_buildable
    write_build_settings
    mkdir('images/not-an-image')

    assert_equal "claude\nplain\n", sndbx('list', 'buildable').out
  end

  def test_list_buildable_defaults_to_the_repo_images
    result = sndbx('list', 'buildable')

    assert_equal Dir.glob('*/Dockerfile', base: REPO_IMAGES_DIR).map { |path| File.dirname(path) }.sort - ['base'],
                 result.out.lines(chomp: true)
    refute File.exist?(settings_path)
  end

  def test_list_running
    write_exec_settings

    assert_equal "gifted_mcnulty\nquirky_hopper\n", sndbx('list', 'running', 'claude').out
    assert_equal "eager_turing\n", sndbx('list', 'running', 'codex').out
    assert_empty sndbx('list', 'running', 'opencode').out
  end

  def test_list_running_without_settings_does_not_create_them
    add_docker_container('claude', 'gifted_mcnulty')

    assert_equal "gifted_mcnulty\n", sndbx('list', 'running', 'claude').out
    refute File.exist?(settings_path)
  end

  def test_list_running_needs_one_image
    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>", sndbx('list', 'running')
    assert_usage_error "unexpected argument 'gifted_mcnulty' found",
                       sndbx('list', 'running', 'claude', 'gifted_mcnulty')
  end

  def test_list_stopped
    write_resume_settings

    assert_equal "sleepy_babbage\n", sndbx('list', 'stopped', 'claude').out
    assert_empty sndbx('list', 'stopped', 'codex').out
  end

  def test_list_stopped_needs_one_image
    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>", sndbx('list', 'stopped')
  end

  def test_list_needs_a_known_list
    assert_usage_error "unrecognized subcommand 'images'", sndbx('list', 'images')
  end
end
