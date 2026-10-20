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

  def test_build_needs_an_image_or_rebuild_base
    write_build_settings

    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>...", sndbx('build')
  end
end
