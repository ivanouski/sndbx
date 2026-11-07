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

    assert_equal [['docker', *run_options('my dir', name: 'my-dir'), '-v', "#{dir}:/workspace/my dir", *config_volume,
                   *TERMINAL_ENV, 'sndbx:claude'],
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
    assert_equal run_options('project'), docker_calls.last.first(6)
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
    assert_equal ['-n', '50', '-f', log_path], JSON.parse(sndbx('logs', 'commands', '-n', '50').out)
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

  def test_logs_follows_the_proxy_log
    write_running_proxy

    result = sndbx('logs', 'proxy', '-n', '50')

    assert_predicate result, :success?
    assert_equal %w[exec sndbx-proxy tail -n 50 -F /var/log/squid/access.log], docker_calls.last
  end

  def test_logs_follows_the_denied_proxy_requests
    write_running_proxy

    assert_predicate sndbx('logs', 'proxy', '--denied'), :success?
    # The script between sh -c and its arguments is tested in proxy.rs
    call = docker_calls.last
    assert_equal %w[exec sndbx-proxy sh -c], call.first(4)
    assert_equal ['sh', '10', '/var/log/squid/access.log', ' TCP_DENIED/'], call.drop(5)
  end

  def test_logs_formats_the_proxy_requests
    write_running_proxy
    File.write(@docker_sandboxes, "my-project 172.18.0.3 2026-11-07T12:00:00.5Z\n")
    File.write(@docker_access_log, <<~LOG)
      2026-11-07T13:03:12+0000 172.18.0.3 TCP_TUNNEL/200 CONNECT api.anthropic.com:443 dur=81234ms in=1520 out=48211
      2026-11-07T13:03:15+0000 172.18.0.7 TCP_DENIED/403 CONNECT evil.example.com:443 dur=0ms in=0 out=3960
      tail: access.log has been replaced
    LOG

    result = sndbx('logs', 'proxy', '--pretty', env: { 'TZ' => 'JST-9' })

    assert_predicate result, :success?
    assert_equal ['2026-11-07 22:03:12  my-project       ok             81s  ↓  47KB  ↑ 1.5KB  api.anthropic.com',
                  '2026-11-07 22:03:15  172.18.0.7       denied        0.0s  ↓ 3.9KB  ↑    0B  evil.example.com',
                  'tail: access.log has been replaced'],
                 result.out.lines(chomp: true)
    assert_equal %w[exec sndbx-proxy tail -n 10 -F /var/log/squid/access.log], docker_execs.last
  end

  def test_logs_fails_when_the_formatted_proxy_log_ends
    write_running_proxy

    result = sndbx('logs', 'proxy', '-p', env: { 'DOCKER_BROKEN' => 'tail' })

    assert_error "can't follow the proxy's log", result
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
