# frozen_string_literal: true

require_relative 'sndbx_test_case'

class ProxyTest < SndbxTestCase
  PROXY_URL = 'http://sndbx-proxy:3128'
  SQUID_IMAGE = 'ubuntu/squid@sha256:6a097f68bae708cedbabd6188d68c7e2e7a38cedd05a176e1cc0ba29e3bbe029'
  PROXY_ARGS = [
    '--network=sndbx-internal',
    *%w[HTTP_PROXY HTTPS_PROXY http_proxy https_proxy].map { |variable| "--env=#{variable}=#{PROXY_URL}" },
    *%w[NO_PROXY no_proxy].map { |variable| "--env=#{variable}=localhost,127.0.0.1,::1" }
  ].freeze

  def test_run_without_proxy_settings_has_no_proxy
    write_claude_settings
    write_allowed_hosts(".openai.com\n")

    sndbx('run', 'claude')

    assert_equal [['info'], %w[image inspect sndbx:claude],
                  ['image', 'inspect', '--format', '{{.Config.User}}', 'sndbx:claude']],
                 docker_calls[0..-2]
    refute File.exist?(squid_conf)
  end

  def test_first_run_starts_the_proxy
    write_proxy_settings
    write_allowed_hosts("# models\n.openai.com\n\npypi.org # packages\n")

    result = sndbx('run', 'claude', '--', '--memory=8g')

    assert_predicate result, :success?
    assert_equal [network_inspect,
                  ['network', 'create', '--internal', 'sndbx-internal'],
                  proxy_inspect,
                  proxy_parse,
                  proxy_run,
                  %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
    assert_includes File.read(squid_conf), "acl allowed dstdomain -n .openai.com pypi.org\n"
    refute_match(/cafe|f00d/, result.out)
    assert_equal [*run_options('project'), *volume('project'), *config_volume, *PROXY_ARGS, '--memory=8g',
                  'sndbx:claude'],
                 docker_calls.last
  end

  def test_second_run_reuses_the_proxy
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    File.delete(@docker_log)

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect], proxy_calls
  end

  def test_changed_hosts_reconfigure_the_proxy
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    File.delete(@docker_log)
    write_allowed_hosts(".openai.com\n.rubygems.org\n")

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_exec_parse, %w[exec sndbx-proxy squid -k reconfigure]],
                 proxy_calls
    assert_includes File.read(squid_conf), "acl allowed dstdomain -n .openai.com .rubygems.org\n"
  end

  def test_a_failed_reconfigure_is_retried
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    write_allowed_hosts(".openai.com\n.rubygems.org\n")

    assert_error "can't start the proxy, see docker logs sndbx-proxy",
                 sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'reconfigure' })
    File.delete(@docker_log)
    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_exec_parse, %w[exec sndbx-proxy squid -k reconfigure]],
                 proxy_calls
  end

  def test_a_proxy_whose_config_file_is_gone_is_replaced
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    FileUtils.rm_rf(File.dirname(squid_conf))
    FileUtils.rm_f(@docker_log)

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_parse, %w[rm -f -v sndbx-proxy], proxy_run,
                  %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
  end

  def test_a_stopped_proxy_is_replaced
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    File.write(@docker_proxy, "false false #{SQUID_IMAGE} #{squid_conf} bridge sndbx-internal")
    File.delete(@docker_log)

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_parse, %w[rm -f -v sndbx-proxy], proxy_run,
                  %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
  end

  def test_a_proxy_from_another_image_is_replaced
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    write_proxy_settings(image: 'ubuntu/squid:6.13')
    File.delete(@docker_log)

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_parse('ubuntu/squid:6.13'), %w[rm -f -v sndbx-proxy],
                  proxy_run('ubuntu/squid:6.13'), %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
  end

  def test_a_missing_allowlist
    write_proxy_settings

    assert_error "no #{allowed_hosts_path}, list the hosts sandboxes behind the proxy may reach there, " \
                 "one per line, or remove proxy from #{settings_path}",
                 sndbx('run', 'claude')
    refute File.exist?(@docker_log)
  end

  def test_an_invalid_proxy_image
    ['', ' ubuntu/squid', 'ubuntu/squid latest', '--privileged'].each do |image|
      write_settings(proxy: { image: image }, images: [{ name: 'claude' }])

      assert_error "invalid #{settings_path}: proxy.image must be an image like ubuntu/squid", sndbx('run', 'claude')
    end
  end

  def test_proxy_image_moved_under_proxy
    write_settings_file("proxy_image: #{SQUID_IMAGE}\nimages:\n  - name: claude\n")

    assert_error "invalid #{settings_path}: proxy_image is now proxy: { image: ... }, see the README",
                 sndbx('run', 'claude')
  end

  def test_allow_all_lets_every_host_through_without_an_allowlist
    write_proxy_settings(allow: 'all')

    assert_predicate sndbx('run', 'claude'), :success?
    config = File.read(squid_conf)
    assert_includes config, "http_access deny to_linklocal\nhttp_access allow all\n"
    refute_includes config, 'dstdomain'
    assert_equal PROXY_ARGS, docker_calls.last[-8..-2]
  end

  def test_allow_all_ignores_the_allowlist
    write_proxy_settings(allow: 'all')
    write_allowed_hosts("open ai.com\n")

    assert_predicate sndbx('run', 'claude'), :success?
    refute_includes File.read(squid_conf), 'dstdomain'
  end

  def test_switching_to_allow_all_reconfigures_the_proxy
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    write_proxy_settings(allow: 'all')
    File.delete(@docker_log)

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_exec_parse, %w[exec sndbx-proxy squid -k reconfigure]],
                 proxy_calls
  end

  def test_an_unknown_allow
    write_proxy_settings(allow: 'any')

    assert_settings_error 'proxy.allow: unknown variant `any`, expected `listed` or `all`', sndbx('run', 'claude')
  end

  def test_a_proxy_with_another_config_file_is_replaced
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    state_home = mkdir('state')
    sndbx('run', 'claude', env: { 'XDG_STATE_HOME' => state_home })
    File.delete(@docker_log)

    sndbx('run', 'claude')

    assert_equal [network_inspect, proxy_inspect, proxy_parse, %w[rm -f -v sndbx-proxy], proxy_run,
                  %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
  end

  def test_a_directory_docker_left_at_the_config_path_is_replaced
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    FileUtils.mkdir_p(squid_conf)

    assert_predicate sndbx('run', 'claude'), :success?
    assert_includes File.read(squid_conf), 'acl allowed dstdomain -n .openai.com'
  end

  def test_a_network_that_is_not_internal
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    File.write(@docker_networks, "sndbx-internal false\n")

    assert_error "the docker network sndbx-internal isn't internal, sandboxes on it could reach any host, " \
                 'remove it with docker network rm sndbx-internal and run again',
                 sndbx('run', 'claude')
    assert_empty docker_runs
  end

  def test_hosts_a_domain_covers_are_left_out
    write_proxy_settings
    write_allowed_hosts("api.github.com\n.github.com\n")

    sndbx('run', 'claude')

    assert_includes File.read(squid_conf), "acl allowed dstdomain -n .github.com\n"
  end

  def test_a_restarting_proxy_is_replaced
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    File.write(@docker_proxy, "true true #{SQUID_IMAGE} #{squid_conf} bridge sndbx-internal")

    assert_predicate sndbx('run', 'claude'), :success?
    assert_equal [network_inspect, %w[network create --internal sndbx-internal], proxy_inspect, proxy_parse,
                  %w[rm -f -v sndbx-proxy], proxy_run, %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
  end

  def test_a_proxy_off_the_internal_network_is_connected
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx-internal' })
    File.delete(@docker_log)

    assert_predicate sndbx('run', 'claude'), :success?
    # The failed run didn't get to record the config as loaded, so this one loads it again
    assert_equal [network_inspect, proxy_inspect, proxy_exec_parse, %w[exec sndbx-proxy squid -k reconfigure],
                  %w[network connect sndbx-internal sndbx-proxy]],
                 proxy_calls
  end

  def test_a_proxy_that_can_not_be_connected
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")

    assert_error "can't connect the proxy sndbx-proxy to the docker network sndbx-internal",
                 sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx-internal' })
    assert_equal [proxy_parse, proxy_run], docker_runs
  end

  def test_a_proxy_that_does_not_start
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")

    assert_error "can't start the proxy, see docker logs sndbx-proxy",
                 sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'sndbx-proxy' })
    assert_equal [proxy_parse, proxy_run], docker_runs
  end

  def test_a_missing_proxy_image_is_pulled
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    File.write(@docker_images, "sndbx:claude ubuntu\n")

    assert_predicate sndbx('run', 'claude'), :success?
    assert_equal [['pull', SQUID_IMAGE], proxy_parse, proxy_run],
                 docker_calls.select { |call| %w[pull run].include?(call.first) }[0..2]
  end

  def test_a_proxy_image_that_can_not_be_pulled
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    File.write(@docker_images, "sndbx:claude ubuntu\n")

    assert_error "can't pull the proxy image #{SQUID_IMAGE}", sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'pull' })
    assert_empty docker_runs
  end

  def test_a_config_squid_can_not_use_is_not_started
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")

    result = sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'parse' })

    refute_predicate result, :success?
    assert_equal "2026/10/22 10:04:58| FATAL: Bungled squid.conf line 4\n" \
                 "Error: squid can't use the proxy's config #{squid_conf}.\n",
                 result.err
    assert_equal [proxy_parse], docker_runs
  end

  def test_a_config_squid_can_not_use_does_not_replace_the_proxy
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    write_proxy_settings(image: 'ubuntu/squid:6.13')
    File.delete(@docker_log)

    refute_predicate sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'parse' }), :success?
    assert_equal [network_inspect, proxy_inspect, proxy_parse('ubuntu/squid:6.13')], proxy_calls
  end

  def test_a_config_squid_can_not_use_is_not_reloaded
    write_proxy_settings
    write_allowed_hosts(".openai.com\n")
    sndbx('run', 'claude')
    write_allowed_hosts(".openai.com\n.rubygems.org\n")
    File.delete(@docker_log)

    refute_predicate sndbx('run', 'claude', env: { 'DOCKER_BROKEN' => 'parse' }), :success?
    assert_equal [network_inspect, proxy_inspect, proxy_exec_parse], proxy_calls
  end

  def test_an_empty_allowlist_denies_everything
    write_proxy_settings
    write_allowed_hosts("# nothing yet\n")

    sndbx('run', 'claude')

    refute_includes File.read(squid_conf), 'allowed'
    assert_equal PROXY_ARGS, docker_calls.last[-8..-2]
  end

  def test_an_image_can_opt_out
    write_settings(proxy: { image: SQUID_IMAGE }, images: [{ name: 'claude', proxy: false }])
    add_docker_image('sndbx:claude')
    write_allowed_hosts(".openai.com\n")

    sndbx('run', 'claude')

    assert_empty proxy_calls
    assert_equal [*run_options('project'), *volume('project'), *config_volume, 'sndbx:claude'], docker_calls.last
  end

  def test_a_proxied_image_can_not_publish_ports
    write_settings(proxy: { image: SQUID_IMAGE }, images: [{ name: 'claude', ports: ['3000'] }])
    add_docker_image('sndbx:claude')
    write_allowed_hosts(".openai.com\n")

    assert_error "image 'claude' publishes ports, which a sandbox behind the proxy can't, " \
                 "set proxy: false for image 'claude' in #{settings_path}",
                 sndbx('run', 'claude')
    write_settings(proxy: { image: SQUID_IMAGE }, images: [{ name: 'claude' }])
    assert_error "-p publishes ports, which a sandbox behind the proxy can't, " \
                 "set proxy: false for image 'claude' in #{settings_path}",
                 sndbx('run', 'claude', '-p', '3000')
    refute File.exist?(@docker_log)
  end

  def test_invalid_allowed_hosts
    write_proxy_settings

    ["open ai.com\n", "a.com\nhttp_access allow all\n", "Example.com\n"].each do |content|
      write_allowed_hosts(content)
      line = content.lines.size
      host = content.lines.last.chomp

      assert_error "invalid #{allowed_hosts_path} line #{line}: '#{host}' is not a host name like " \
                   '.openai.com or pypi.org',
                   sndbx('run', 'claude')
    end
    refute File.exist?(@docker_log)
  end

  def test_proxy_must_be_a_boolean
    write_settings(proxy: { image: SQUID_IMAGE }, images: [{ name: 'claude', proxy: 'no' }])

    result = sndbx('run', 'claude')

    refute_predicate result, :success?
    assert_includes result.err, 'images[0].proxy: invalid type'
  end

  private

  def write_allowed_hosts(content)
    FileUtils.mkdir_p(File.dirname(allowed_hosts_path))
    File.write(allowed_hosts_path, content)
  end

  def allowed_hosts_path
    File.join(@home, '.config', 'sndbx', 'allowed_hosts')
  end

  def squid_conf
    File.join(@home, '.local', 'state', 'sndbx', 'squid.conf')
  end

  def network_inspect
    ['network', 'inspect', '--format', '{{.Internal}}', 'sndbx-internal']
  end

  def proxy_inspect
    ['container', 'inspect', '--format',
     "{{.State.Running}}\n{{.State.Restarting}}\n{{.Config.Image}}\n" \
     "{{range $name, $network := .NetworkSettings.Networks}}{{$name}} {{end}}\n{{range .Mounts}}{{.Source}}\n{{end}}",
     'sndbx-proxy']
  end

  def proxy_parse(image = SQUID_IMAGE)
    ['run', '--rm', '--entrypoint=squid', '-v', "#{squid_conf}:/etc/squid/squid.conf:ro", image, '-k', 'parse']
  end

  def proxy_exec_parse
    %w[exec sndbx-proxy squid -k parse]
  end

  def proxy_run(image = SQUID_IMAGE)
    ['run', '-d', '--name', 'sndbx-proxy', '--restart=unless-stopped',
     '-v', "#{squid_conf}:/etc/squid/squid.conf:ro", image]
  end

  def write_proxy_settings(image: SQUID_IMAGE, **proxy)
    write_settings(proxy: { image: image, **proxy }, images: [{ name: 'claude' }])
    add_docker_image('sndbx:claude')
    add_docker_image(image, user: 'proxy')
  end

  # The docker calls between checking the image and running the sandbox
  def proxy_calls
    docker_calls.reject do |call|
      call == ['info'] || call.first(2) == %w[image inspect] || call.first(3) == %w[run --rm -it]
    end
  end
end
