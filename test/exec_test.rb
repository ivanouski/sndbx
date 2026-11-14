# frozen_string_literal: true

require_relative 'sndbx_test_case'
require 'pty'

class ExecTest < SndbxTestCase
  def test_exec
    write_exec_settings

    sndbx('exec', 'gifted_mcnulty', 'bash')

    assert_equal [['info'], ['ps', '--filter', 'label=sndbx.image', '--format', '{{.Names}}'],
                  ['exec', '-i', 'gifted_mcnulty', 'bash']],
                 docker_calls
  end

  def test_exec_as_user
    write_exec_settings

    sndbx('exec', '-u', 'root', 'gifted_mcnulty', 'bash')
    sndbx('exec', 'gifted_mcnulty', '--user=root', 'bash')
    sndbx('exec', 'gifted_mcnulty', '-u', 'root', 'bash')

    assert_equal [['exec', '-i', '--user=root', 'gifted_mcnulty', 'bash']] * 3, docker_execs
  end

  def test_exec_passes_options_after_the_command_to_it
    write_exec_settings

    sndbx('exec', 'gifted_mcnulty', 'ls', '-la', '-u', 'x', '--help', '--')

    assert_equal [['exec', '-i', 'gifted_mcnulty', 'ls', '-la', '-u', 'x', '--help', '--']], docker_execs
  end

  def test_exec_with_a_terminal
    write_exec_settings

    PTY.spawn({ 'HOME' => @home, 'PATH' => @bin, 'DOCKER_LOG' => @docker_log, 'DOCKER_IMAGES' => @docker_images,
                'DOCKER_CONTAINERS' => @docker_containers, 'DOCKER_NETWORKS' => @docker_networks,
                'DOCKER_PROXY' => @docker_proxy, 'DOCKER_SANDBOXES' => @docker_sandboxes,
                'DOCKER_ACCESS_LOG' => @docker_access_log },
              *COMMAND, 'exec', 'gifted_mcnulty', 'bash', chdir: @work) do |out, _in, pid|
      begin
        out.read
      rescue Errno::EIO
        # the pty closes when sndbx exits
      end
      Process.wait(pid)
    end

    assert_equal [['exec', '-it', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_after_double_dash
    write_exec_settings

    sndbx('exec', 'gifted_mcnulty', '--', '-weird')
    sndbx('exec', '--', 'gifted_mcnulty', 'bash')

    assert_equal [['exec', '-i', 'gifted_mcnulty', '-weird'], ['exec', '-i', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_sandbox_of_any_image
    write_exec_settings

    sndbx('exec', 'eager_turing', 'bash')

    assert_equal [['exec', '-i', 'eager_turing', 'bash']], docker_execs
  end

  def test_exec_without_settings
    add_docker_container('claude', 'gifted_mcnulty')

    sndbx('exec', 'gifted_mcnulty', 'bash')

    assert_equal [['exec', '-i', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_not_a_sandbox
    write_resume_settings
    add_docker_container('-', 'by_hand')

    assert_error "container 'by_hand' is not a running sandbox, " \
                 'expected one of: gifted_mcnulty, eager_turing, quirky_hopper',
                 sndbx('exec', 'by_hand', 'bash')
    assert_error "container 'sleepy_babbage' is not a running sandbox, " \
                 'expected one of: gifted_mcnulty, eager_turing, quirky_hopper',
                 sndbx('exec', 'sleepy_babbage', 'bash')
    assert_empty docker_execs
  end

  def test_exec_without_running_containers
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])

    assert_error 'no running sandbox', sndbx('exec', 'gifted_mcnulty', 'bash')
  end
end
