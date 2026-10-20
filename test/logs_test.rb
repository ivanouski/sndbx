# frozen_string_literal: true

require_relative 'sndbx_test_case'
require 'shellwords'

class LogsTest < SndbxTestCase
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

    sndbx('list', 'configured')
    sndbx('list', 'buildable')
    sndbx('list', 'running', 'claude')
    sndbx('list', 'stopped', 'claude')

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

  def test_logs_under_xdg_state_home
    write_fake_tail
    write_claude_settings
    state_home = File.join(@root, 'xdg-state')
    path = File.join(state_home, 'sndbx', 'sndbx.log')

    sndbx('run', 'claude', env: { 'XDG_STATE_HOME' => state_home })

    assert_equal 1, File.readlines(path).size
    refute File.exist?(log_path)
    assert_equal ['-n', '10', '-f', path], JSON.parse(sndbx('logs', env: { 'XDG_STATE_HOME' => state_home }).out)
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
    assert_usage_error "invalid value 'x' for '--lines <N>': expected a number", sndbx('logs', '-n', 'x')
    assert_usage_error "invalid value '-1' for '--lines <N>': expected a line count of 0 or more",
                       sndbx('logs', '-n', '-1')
  end

  def test_logs_takes_no_arguments
    assert_usage_error "unexpected argument 'claude' found", sndbx('logs', 'claude')
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
end
