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

    assert_equal [['docker', *run_options('my dir'), '-v', "#{dir}:/workspace/my dir", *config_volume, *TERMINAL_ENV,
                   'sndbx:claude'],
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
    result = sndbx('logs', 'commands', env: { 'XDG_STATE_HOME' => state_home })
    assert_equal ['-n', '10', '-f', path], JSON.parse(result.out)
  end

  def test_logs_follows_the_log
    write_fake_tail
    FileUtils.mkdir_p(File.dirname(log_path))
    File.write(log_path, '')

    result = sndbx('logs', 'commands')

    assert_predicate result, :success?
    assert_equal ['-n', '10', '-f', log_path], JSON.parse(result.out)
  end

  def test_logs_with_lines
    write_fake_tail
    FileUtils.mkdir_p(File.dirname(log_path))
    File.write(log_path, '')

    assert_equal ['-n', '50', '-f', log_path], JSON.parse(sndbx('logs', 'commands', '-n', '50').out)
    assert_equal ['-n', '0', '-f', log_path], JSON.parse(sndbx('logs', 'commands', '--lines=0').out)
  end

  def test_logs_without_a_log
    write_fake_tail

    assert_error "no log yet at #{log_path}", sndbx('logs', 'commands')
  end

  def test_logs_without_tail
    FileUtils.mkdir_p(File.dirname(log_path))
    File.write(log_path, '')

    assert_error 'tail is not installed', sndbx('logs', 'commands')
  end

  def test_logs_needs_a_line_count
    assert_usage_error "invalid value 'x' for '--lines <N>': expected a number", sndbx('logs', 'commands', '-n', 'x')
    assert_usage_error "invalid value '-1' for '--lines <N>': expected a line count of 0 or more",
                       sndbx('logs', 'commands', '-n', '-1')
  end

  def test_logs_takes_no_arguments
    assert_usage_error "unexpected argument 'claude' found", sndbx('logs', 'commands', 'claude')
    assert_usage_error "unrecognized subcommand 'claude'", sndbx('logs', 'claude')
  end

  def test_logs_follows_the_proxy_log
    write_running_proxy

    result = sndbx('logs', 'proxy', '-n', '50')

    assert_predicate result, :success?
    assert_equal %w[exec sndbx-proxy tail -n 50 -F /var/log/squid/access.log], docker_calls.last
  end

  def test_logs_follows_the_denied_proxy_requests
    write_running_proxy

    assert_predicate sndbx('logs', 'proxy', '--denied'), :success?
    exec, container, sh, c, script, name, *args = docker_calls.last
    assert_equal %w[exec sndbx-proxy sh -c], [exec, container, sh, c]
    assert_equal ['sh', '10', '/var/log/squid/access.log', ' TCP_DENIED/'], [name, *args]

    log = File.join(@root, 'access.log')
    lines = (1..12).map { |i| "2026-10-21T14:02:#{i}+0000 172.18.0.2 TCP_DENIED/403 CONNECT host#{i}:443" }
    File.write(log, "#{lines.join("\n")}\n2026-10-21T14:03:00+0000 172.18.0.2 TCP_TUNNEL/200 CONNECT ok:443\n")
    # The real tail, which with -F only reads what's there
    real_tail = ENV.fetch('PATH').split(':').map { |dir| File.join(dir, 'tail') }.find { |path| File.executable?(path) }
    write_executable(File.join(@bin, 'tail'),
                     "#!/bin/sh\n[ \"$3\" = -F ] && exec #{real_tail} \"$1\" \"$2\" \"$4\"\nexec #{real_tail} \"$@\"\n")
    env = { 'PATH' => "#{@bin}:#{ENV.fetch('PATH')}" }
    out = IO.popen(env, ['sh', '-c', script, name, '2', log, args.last], &:read)

    assert_equal "#{lines[-2..].join("\n")}\n", out
  end

  def test_logs_without_a_running_proxy
    assert_error "the proxy sndbx-proxy isn't running, sndbx run starts it", sndbx('logs', 'proxy')
    File.write(@docker_proxy, 'false false 0 squid /x bridge')
    assert_error "the proxy sndbx-proxy isn't running, sndbx run starts it", sndbx('logs', 'proxy')
  end

  private

  def write_running_proxy
    File.write(@docker_proxy, 'true false 0 squid /x bridge sndbx-internal')
  end

  def write_fake_tail
    write_executable(File.join(@bin, 'tail'), "#!#{RbConfig.ruby} --disable-gems\nrequire 'json'\nputs ARGV.to_json\n")
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
