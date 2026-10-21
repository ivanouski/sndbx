# frozen_string_literal: true

require_relative 'sndbx_test_case'

class CliTest < SndbxTestCase
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
    assert_help 'Usage: sndbx build [--rebuild-base] IMAGE...', sndbx('build', '--help')
  end

  def test_exec_help
    assert_help 'Usage: sndbx exec IMAGE CONTAINER [options] COMMAND [ARGS]', sndbx('exec', '-h')
  end

  def test_resume_help
    assert_help 'Usage: sndbx resume IMAGE CONTAINER', sndbx('resume', '-h')
  end

  def test_list_help
    assert_help 'Usage: sndbx list WHAT', sndbx('list', '-h')
    assert_help 'Usage: sndbx list WHAT', sndbx('list')
    assert_help 'Usage: sndbx list running IMAGE', sndbx('list', 'running', '-h')
  end

  def test_logs_help
    assert_help 'Usage: sndbx logs WHAT [options]', sndbx('logs', '-h')
    assert_help 'Usage: sndbx logs WHAT [options]', sndbx('logs')
    assert_help 'Usage: sndbx logs commands [options]', sndbx('logs', 'commands', '-h')
    assert_help 'Usage: sndbx logs proxy [options]', sndbx('logs', 'proxy', '-h')
  end

  def test_version
    result = sndbx('-V')

    assert_predicate result, :success?
    assert_match(/\Asndbx \d+\.\d+\.\d+\n\z/, result.out)
  end

  def test_unknown_command
    assert_usage_error "unrecognized subcommand 'foo'", sndbx('foo')
  end

  def test_invalid_option
    assert_usage_error "unexpected argument '--bogus' found", sndbx('--bogus')
  end

  def test_missing_home
    missing = File.join(@root, 'missing')

    assert_predicate sndbx('-h', env: { 'HOME' => missing }), :success?
    assert_predicate sndbx('list', 'configured', env: { 'HOME' => missing }), :success?
  end

  def test_old_list_options_are_gone
    assert_usage_error "unexpected argument '--list' found", sndbx('run', '--list')
    assert_usage_error "unexpected argument '--list' found", sndbx('build', '--list')
    assert_usage_error "unexpected argument '--list' found", sndbx('exec', 'claude', '--list')
    assert_usage_error "unexpected argument '--list' found", sndbx('resume', 'claude', '--list')
  end
end
