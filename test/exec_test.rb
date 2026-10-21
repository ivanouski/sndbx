# frozen_string_literal: true

require_relative 'sndbx_test_case'
require 'pty'

class ExecTest < SndbxTestCase
  def test_exec
    write_exec_settings

    sndbx('exec', 'claude', 'gifted_mcnulty', 'bash')

    assert_equal [['info'], ['ps', '--filter', 'label=sndbx.image=claude', '--format', '{{.Names}}'],
                  ['exec', '-i', 'gifted_mcnulty', 'bash']],
                 docker_calls
  end

  def test_exec_as_user
    write_exec_settings

    sndbx('exec', '-u', 'root', 'claude', 'gifted_mcnulty', 'bash')
    sndbx('exec', 'claude', '--user=root', 'gifted_mcnulty', 'bash')
    sndbx('exec', 'claude', 'gifted_mcnulty', '-u', 'root', 'bash')

    assert_equal [['exec', '-i', '--user=root', 'gifted_mcnulty', 'bash']] * 3, docker_execs
  end

  def test_exec_passes_options_after_the_command_to_it
    write_exec_settings

    sndbx('exec', 'claude', 'gifted_mcnulty', 'ls', '-la', '-u', 'x', '--help', '--')

    assert_equal [['exec', '-i', 'gifted_mcnulty', 'ls', '-la', '-u', 'x', '--help', '--']], docker_execs
  end

  def test_exec_with_a_terminal
    write_exec_settings

    PTY.spawn({ 'HOME' => @home, 'PATH' => @bin, 'DOCKER_LOG' => @docker_log, 'DOCKER_IMAGES' => @docker_images,
                'DOCKER_CONTAINERS' => @docker_containers, 'DOCKER_NETWORKS' => @docker_networks,
                'DOCKER_PROXY' => @docker_proxy },
              *COMMAND, 'exec', 'claude', 'gifted_mcnulty', 'bash', chdir: @work) do |out, _in, pid|
      begin
        out.read
      rescue Errno::EIO
        # the pty closes when sndbx exits
      end
      Process.wait(pid)
    end

    assert_equal [['exec', '-it', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_needs_a_nonempty_user
    write_exec_settings

    assert_usage_error "a value is required for '--user <USER>'",
                       sndbx('exec', '-u', '', 'claude', 'gifted_mcnulty', 'bash')
    assert_usage_error "a value is required for '--user <USER>'",
                       sndbx('exec', '--user=', 'claude', 'gifted_mcnulty', 'bash')
    refute File.exist?(@docker_log)
  end

  def test_exec_after_double_dash
    write_exec_settings

    sndbx('exec', 'claude', 'gifted_mcnulty', '--', '-weird')
    sndbx('exec', '--', 'claude', 'gifted_mcnulty', 'bash')

    assert_equal [['exec', '-i', 'gifted_mcnulty', '-weird'], ['exec', '-i', 'gifted_mcnulty', 'bash']], docker_execs
  end

  def test_exec_needs_an_image_a_container_and_a_command
    write_exec_settings

    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>", sndbx('exec')
    assert_usage_error "the following required arguments were not provided:\n  <CONTAINER>", sndbx('exec', 'claude')
    assert_usage_error "the following required arguments were not provided:\n  <COMMAND>...",
                       sndbx('exec', 'claude', 'gifted_mcnulty')
  end

  def test_exec_unknown_image
    write_exec_settings

    assert_error "unknown image 'nope', expected one of: claude, codex", sndbx('exec', 'nope', 'gifted_mcnulty', 'bash')
  end

  def test_exec_container_of_another_image
    write_exec_settings

    assert_error "container 'eager_turing' is not a running claude sandbox, " \
                 'expected one of: gifted_mcnulty, quirky_hopper',
                 sndbx('exec', 'claude', 'eager_turing', 'bash')
    assert_empty docker_execs
  end

  def test_exec_without_running_containers
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }])

    assert_error 'no running claude sandbox', sndbx('exec', 'claude', 'gifted_mcnulty', 'bash')
  end

  def test_exec_without_docker
    write_exec_settings

    assert_error 'docker is not installed', sndbx('exec', 'claude', 'gifted_mcnulty', 'bash', docker: false)
  end
end
