# frozen_string_literal: true

require_relative 'sndbx_test_case'
require 'io/console'
require 'pty'
require 'time'
require 'timeout'

# sndbx top with --once, which prints a frame, and in a pseudo-terminal for the keys
class TopTest < SndbxTestCase
  # The frame's times are UTC
  TOP_ENV = { 'TZ' => 'UTC' }.freeze
  PROXY_IMAGE = 'ubuntu/squid'

  def test_dashboard
    write_top_settings

    result = top

    assert_predicate result, :success?, result.err
    assert_equal <<~FRAME, result.out
       sndbx  [1 Dashboard]  2 Images                        docker ✓ │ proxy ✓ allows listed
      ┌ Sandboxes ───────────────────────────────────────────────────────────────────────────┐
      │ NAME        IMAGE    STATE         UP     CPU     MEM      PIDS  NET ↑  NET ↓  EGRESS│
      │▶api-1       codex    running      14m    0.3%    201M   38/4096  12.3K   3.1M  proxy │
      │ dsh         dsh      running       3d    1.2%    340M        61   230K   5.6M  open  │
      │ my-project  claude   running    2h13m    4.1%    612M  143/4096   1.2M    48M  proxy │
      │ old-thing   claude   stopped                                                   proxy │
      ├ api-1 · 172.18.0.4 ──────────────────────────────────────────────────────────────────┤
      │ mounts   ~/src/api → /workspace/api (workdir)                                        │
      │ config   ~/.config/sndbx/codex → /home/ubuntu/.codex                                 │
      │ hosts    sentry.io 2 · github.com 1                                                  │
      └──────────────────────────────────────────────────────────────────────────────────────┘
      ┌ Proxy log · all · 4 denied ──────────────────────────────────────────────────────────┐
      │#{shown(3600)} 172.18.0.4   200 nexus.example.com:443                                       │
      │#{shown(120)} api-1        200 github.com:443                                              │
      │#{shown(113)} my-project   200 api.anthropic.com:443                                       │
      │#{shown(106)} api-1        403 sentry.io:443                                               │
      │#{shown(102)} my-project   403 registry.yarnpkg.com:443                                    │
      │#{shown(96)} api-1        403 sentry.io:443                                               │
      │#{shown(93)} 172.18.0.9   403 telemetry.x.dev                                             │
      │                                                                                      │
      │                                                                                      │
      └──────────────────────────────────────────────────────────────────────────────────────┘
       tab focus  enter shell  s stop  d remove  f filter  z zoom  q quit
    FRAME
  end

  def test_images
    write_top_settings

    result = top('--view', 'images')

    assert_predicate result, :success?, result.err
    assert_equal <<~FRAME, result.out
       sndbx   1 Dashboard  [2 Images]                       docker ✓ │ proxy ✓ allows listed
      ┌ Images ──────────────────────────────────────────────────────────────────────────────┐
      │ NAME      TAG          SIZE   BUILT     SANDBOXES        NET    PORTS                │
      │▶claude    sndbx:claude 1.9G   2d        1 up, 1 stopped  proxy  —                    │
      │ codex     sndbx:codex  1.7G   9d ⚠      1 up             proxy  —                    │
      │ dsh       sndbx:dsh    1.1G   3d        1 up             open   127.0.0.1:3080:3080  │
      │ base      sndbx:base   1.4G   5d                                                     │
      │ cursor    sndbx:cursor        not built not in settings                              │
      └──────────────────────────────────────────────────────────────────────────────────────┘
      ┌ claude ──────────────────────────────────────────────────────────────────────────────┐
      │ source       ~/images/claude/Dockerfile                                              │
      │ config       ~/.config/sndbx/claude → ~/.claude                                      │
      │ user         the image's                                                             │
      │ docker_args  --security-opt=no-new-privileges --memory=8g                            │
      │ sandboxes    my-project (running 2h13m), old-thing (stopped)                         │
      │                                                                                      │
      │                                                                                      │
      │                                                                                      │
      │                                                                                      │
      │                                                                                      │
      │                                                                                      │
      │                                                                                      │
      └──────────────────────────────────────────────────────────────────────────────────────┘
       b build  B rebuild with base  d remove  enter show sandboxes  1 dashboard  q quit
    FRAME
  end

  def test_hosts_table_on_a_wide_terminal
    write_top_settings
    File.write(@docker_access_log, (1..12).map { |n| access_line('172.18.0.4', "host#{n}.example.com", n) }.join,
               mode: 'a')

    lines = top('--size', '150x40').out.lines

    header = 'HOST                  REQ  DENIED   NET ↓'
    hosts = lines.find { |line| line.include?(' hosts ') }

    assert_equal 2, hosts.scan(header).size, hosts
    assert_includes lines[lines.index(hosts) + 1], 'host12.example.com     12            1.2K    host5.example.com '
    assert_includes lines.join, 'sentry.io               2       2'
  end

  def test_sandbox_details_without_the_proxy
    write_top_settings
    File.write(@docker_containers, "dsh dsh running sndbx:dsh\n")

    out = top.out

    assert_includes out, '├ dsh · 172.17.0.2 ─'
    assert_includes out, '│ mounts   ~/src/notes → /workspace/notes (workdir)   '
    assert_includes out, '│ config   ~/.config/sndbx/dsh → /home/ubuntu/.dsh   '
    assert_includes out, '│ ports    127.0.0.1:3080:3080   '
    assert_includes out, '│ hosts    not behind the proxy   '
  end

  def test_without_sandboxes_or_proxy
    write_settings(images: [{ name: 'claude' }])

    out = top.out

    assert_includes out, 'docker ✓ │ proxy off'
    assert_includes out, 'No sandboxes, sndbx run starts one'
    assert_includes out, '│The proxy is off, set proxy in ~/.config/sndbx/settings.yml to turn it on   '
    refute_includes out, 'denied'
  end

  def test_proxy_down
    write_settings(proxy: { image: PROXY_IMAGE }, images: [{ name: 'claude' }])

    out = top.out

    assert_includes out, 'proxy ✗ allows listed'
    assert_includes out, "sndbx-proxy isn't running, sndbx run starts it"
    refute(docker_calls.any? { |call| call.first(2) == %w[exec sndbx-proxy] })
  end

  def test_without_docker
    write_settings(images: [{ name: 'claude' }])

    out = sndbx('top', '--once', docker: false).out

    assert_includes out, 'docker ✗'
    assert_includes out, 'docker is not installed'
  end

  def test_settings_error
    write_settings_file("images: nope\n")

    dashboard = top.out
    images = top('--view', 'images').out

    assert_includes dashboard, 'settings ✗ │ docker ✓'
    assert dashboard.lines.last.start_with?(' invalid /'), dashboard.lines.last
    assert_includes images, 'invalid'
  end

  def test_size_below_the_smallest
    result = top('--size', '79x30')

    assert_usage_error "invalid value '79x30' for '--size <WIDTHxHEIGHT>': expected a size like 88x24, of 80x20 " \
                       'at least', result
  end

  def test_does_not_log_reading_docker
    write_top_settings

    top
    top('--view', 'images')

    refute File.exist?(File.join(@home, '.local', 'state', 'sndbx', 'sndbx.log'))
  end

  def test_help
    result = sndbx('top', '--help')

    assert_predicate result, :success?
    assert_equal "Usage: sndbx top [options]\n", result.out.lines[2]
  end

  def test_usage_error
    result = sndbx('top', '--size', '80x20')

    assert_usage_error 'the following required arguments were not provided', result
  end

  def test_empty_size
    result = top('--size', '0x24')

    assert_usage_error "invalid value '0x24' for '--size <WIDTHxHEIGHT>': expected a size like 88x24, of 80x20 " \
                       'at least', result
  end

  def test_offers_building_only_images_in_images_dir
    images_dir = File.join(@home, 'images')
    FileUtils.mkdir_p(File.join(images_dir, 'base'))
    write_settings(images_dir: images_dir, images: [{ name: 'pulled' }])

    footer = top('--view', 'images').out.lines.last

    refute_includes footer, 'build'
    assert_includes footer, 'd remove'
  end

  def test_without_sndbx_top
    bin = mkdir('sndbx-bin')
    FileUtils.cp(COMMAND.first, bin)

    result = Result.new(*Open3.capture3({ 'HOME' => @home, 'PATH' => @bin, **XDG_ENV, **BUNDLER_ENV },
                                        File.join(bin, 'sndbx'), 'top', chdir: @work))

    assert_error 'sndbx-top is not installed, put it next to sndbx or in PATH: download it from the latest ' \
                 'release, or run cargo install --path top in a clone', result
  end

  def test_follows_clicks_and_gives_the_mouse_back
    write_top_settings

    output = interact do |input, screen|
      wait_for(screen, 'sentry.io')
      # A click, pressed and released, on the Images tab, in SGR's 1-based columns and rows
      input.write("\e[<0;25;1M\e[<0;25;1m")
      # Builds the first image, claude, which only the Images view does
      input.write('b')
      wait_for(screen, 'Press enter to return to sndbx top')
      input.write("\r")
      input.write('q')
    end

    assert_includes docker_builds.map { |call| call[-2] }, 'sndbx:claude'

    assert_includes output, "\e[?1000h\e[?1006h"
    assert output.rindex("\e[?1006l\e[?1000l") > output.rindex("\e[?1000h"), 'the mouse is still captured'
  end

  def test_opens_a_shell_with_the_cursor_shown
    write_top_settings

    output = interact do |input, screen|
      wait_for(screen, 'api-1')
      input.write("\r")
      wait_until { docker_execs.any? { |call| call.include?('api-1') } }
      input.write('q')
    end

    assert_includes docker_execs.find { |call| call.include?('api-1') }, 'sh'
    # Shown for the shell, then again on exit
    assert_operator output.scan("\e[?25h").size, :>=, 2
  end

  def test_stops_a_sandbox_after_asking
    write_top_settings

    interact do |input, screen|
      wait_for(screen, 'api-1')
      input.write('s')
      wait_for(screen, 'Stop api-1?')
      input.write('y')
      wait_until { docker_calls.include?(%w[stop api-1]) }
      input.write('q')
    end

    assert_includes docker_calls, %w[stop api-1]
  end

  def test_quits_on_sigterm_like_on_q
    write_top_settings
    killed_at = nil

    output = interact do |_input, screen, pid|
      wait_for(screen, 'api-1')
      wait_until('docker events to start') { File.size?(@docker_events_pid) }
      killed_at = screen.size
      Process.kill('TERM', pid)
    end

    after = output[killed_at..]

    assert_includes after, "\e[?1006l\e[?1000l", 'the mouse is still captured'
    assert_includes after, "\e[?1049l", 'the alternate screen is still on'
    events = File.read(@docker_events_pid).to_i
    wait_until('docker events to stop') { !running?(events) }
  end

  private

  def top(*args)
    sndbx('top', '--once', *args, env: TOP_ENV)
  end

  # Sandboxes of three images, one stopped, a running proxy with a log, and images, base newer than codex
  def write_top_settings
    images_dir = File.join(@home, 'images')
    FileUtils.mkdir_p(images_dir)
    write_settings(images_dir: images_dir, docker_args: ['--security-opt=no-new-privileges'],
                   proxy: { image: PROXY_IMAGE },
                   images: [{ name: 'claude', docker_args: ['--memory=8g'] }, { name: 'codex' },
                            { name: 'dsh', ports: ['127.0.0.1:3080:3080'], proxy: false }])
    %w[base claude codex dsh cursor].each do |name|
      File.write(File.join(FileUtils.mkdir_p(File.join(images_dir, name)).first, 'Dockerfile'), "FROM ubuntu\n")
    end
    write_top_docker
  end

  def write_top_docker
    now = @now = Time.now.utc
    [PROXY_IMAGE, 'sndbx:base', 'sndbx:claude', 'sndbx:codex', 'sndbx:dsh'].each { |tag| add_docker_image(tag) }
    add_docker_container('claude', 'my-project')
    add_docker_container('codex', 'api-1')
    add_docker_container('dsh', 'dsh')
    add_docker_container('claude', 'old-thing', 'exited')
    squid_conf = File.join(@home, '.local', 'state', 'sndbx', 'squid.conf')
    File.write(@docker_proxy, "true false #{PROXY_IMAGE} #{squid_conf} bridge sndbx-internal")
    dsh = sandbox_details(now - (3 * 86_400) - 60, '172.17.0.2', 'dsh', %w[notes], network: 'bridge', port: 3080)
    write_json(@docker_inspect,
               'my-project' => sandbox_details(now - 7990, '172.18.0.3', 'claude', %w[api web]),
               'api-1' => sandbox_details(now - 850, '172.18.0.4', 'codex', %w[api]),
               'dsh' => dsh,
               'old-thing' => { 'NetworkSettings' => { 'Networks' => { 'sndbx-internal' => { 'IPAddress' => '' } } } })
    write_json(@docker_stats,
               'my-project' => stats('4.10%', '612MiB', '143', '48MB / 1.2MB'),
               'api-1' => stats('0.30%', '201MiB', '38', '3.1MB / 12.3kB'),
               'dsh' => stats('1.20%', '340MiB', '61', '5.6MB / 230kB'))
    write_json(@docker_image_details,
               'sndbx:base' => image_details('1.4GB', now, 5),
               'sndbx:claude' => image_details('1.9GB', now, 2),
               'sndbx:codex' => image_details('1.7GB', now, 9),
               'sndbx:dsh' => image_details('1.1GB', now, 3))
    # The first from api-1's address before it started, by a sandbox that's gone
    File.write(@docker_access_log, <<~LOG)
      #{logged(3600)} 172.18.0.4 TCP_TUNNEL/200 CONNECT nexus.example.com:443 dur=10ms in=100 out=1000
      #{logged(120)} 172.18.0.4 TCP_TUNNEL/200 CONNECT github.com:443 dur=812ms in=12000 out=3100000
      #{logged(113)} 172.18.0.3 TCP_TUNNEL/200 CONNECT api.anthropic.com:443 dur=9000ms in=1200000 out=48000000
      #{logged(106)} 172.18.0.4 TCP_DENIED/403 CONNECT sentry.io:443 dur=0ms in=0 out=0
      #{logged(102)} 172.18.0.3 TCP_DENIED/403 CONNECT registry.yarnpkg.com:443 dur=0ms in=0 out=0
      #{logged(96)} 172.18.0.4 TCP_DENIED/403 CONNECT sentry.io:443 dur=0ms in=0 out=0
      #{logged(93)} 172.18.0.9 TCP_DENIED/403 GET http://telemetry.x.dev/v1 dur=0ms in=0 out=0
      not a line in sndbx's format
    LOG
  end

  # Started at `started`, with `dirs` of ~/src mounted in /workspace, starting in the last, and the image's config;
  # with a pids limit or a port published on localhost, with a pids limit or a port published on localhost
  def access_line(client, host, requests)
    "#{logged(30)} #{client} TCP_TUNNEL/200 CONNECT #{host}:443 dur=1ms in=10 out=100\n" * requests
  end

  # The log's time `seconds` before the fixtures were written
  def logged(seconds)
    (@now - seconds).strftime('%FT%T+0000')
  end

  # As the log pane shows it, in UTC
  def shown(seconds)
    (@now - seconds).strftime('%T')
  end

  def sandbox_details(started, ip, image, dirs, network: 'sndbx-internal', port: nil)
    mounts = dirs.map { |dir| { 'Source' => File.join(@home, 'src', dir), 'Destination' => "/workspace/#{dir}" } }
    config = { 'Source' => File.join(@home, '.config', 'sndbx', image), 'Destination' => "/home/ubuntu/.#{image}" }
    host_config = if port
                    { 'PortBindings' => { "#{port}/tcp" => [{ 'HostIp' => '127.0.0.1', 'HostPort' => port.to_s }] } }
                  else
                    { 'PidsLimit' => 4096 }
                  end
    { 'State' => { 'StartedAt' => started.iso8601 }, 'HostConfig' => host_config,
      'Config' => { 'WorkingDir' => "/workspace/#{dirs.last}" }, 'Mounts' => [*mounts, config],
      'NetworkSettings' => { 'Networks' => { network => { 'IPAddress' => ip.to_s } } } }
  end

  # The network's bytes received / sent
  def stats(cpu, memory, pids, network)
    { 'CPUPerc' => cpu, 'MemUsage' => "#{memory} / 7.6GiB", 'PIDs' => pids, 'NetIO' => network }
  end

  def image_details(size, now, days)
    { 'Size' => size, 'CreatedAt' => (now - (days * 86_400) - 3600).strftime('%Y-%m-%d %H:%M:%S +0000 UTC') }
  end

  def write_json(path, content)
    File.write(path, JSON.generate(content))
  end

  # Runs sndbx top in an 88x24 pseudo-terminal, yields its input, what it drew so far and its pid, and returns all
  # it drew once it exits
  def interact
    env = { 'HOME' => @home, 'PATH' => @bin, 'TERM' => 'xterm-256color', **TOP_ENV, **XDG_ENV, **BUNDLER_ENV,
            'DOCKER_LOG' => @docker_log, 'DOCKER_IMAGES' => @docker_images, 'DOCKER_CONTAINERS' => @docker_containers,
            'DOCKER_NETWORKS' => @docker_networks, 'DOCKER_PROXY' => @docker_proxy, 'DOCKER_INSPECT' => @docker_inspect,
            'DOCKER_STATS' => @docker_stats, 'DOCKER_IMAGE_DETAILS' => @docker_image_details,
            'DOCKER_ACCESS_LOG' => @docker_access_log, 'DOCKER_EVENTS_PID' => @docker_events_pid }
    screen = +''
    PTY.spawn(env, *COMMAND, 'top', chdir: @work) do |output, input, pid|
      input.winsize = [24, 88]
      reader = Thread.new do
        loop { screen << output.readpartial(4096) }
      rescue EOFError, Errno::EIO
        nil
      end
      yield input, screen, pid
      _pid, status = Timeout.timeout(10) { Process.wait2(pid) }

      assert_predicate status, :success?, screen[-500..]
      reader.join(1)
    end
    screen
  end

  # Whether the process runs, not counting a zombie its new parent didn't reap yet
  def running?(pid)
    Process.kill(0, pid)
    stat = "/proc/#{pid}/stat"
    !File.exist?(stat) || File.read(stat)[/\) (\S)/, 1] != 'Z'
  rescue Errno::ESRCH, Errno::ENOENT
    false
  end

  # Text drawn at once, as later frames draw only the cells that changed
  def wait_for(screen, text)
    wait_until("#{text.inspect} to show, the screen ends with #{screen[-500..].inspect}") { screen.include?(text) }
  end

  def wait_until(what = 'the condition')
    Timeout.timeout(10) { sleep 0.05 until yield }
  rescue Timeout::Error
    flunk "Timed out waiting for #{what}"
  end
end
