# frozen_string_literal: true

require_relative 'sndbx_test_case'

class ResumeTest < SndbxTestCase
  def test_resume
    write_resume_settings

    sndbx('resume', 'claude', 'sleepy_babbage')

    assert_equal [['info'],
                  ['ps', '--all', '--filter', 'status=exited', '--filter', 'label=sndbx.image=claude',
                   '--format', '{{.Names}}'],
                  ['start', '-ai', 'sleepy_babbage']],
                 docker_calls
  end

  def test_resume_running_container
    write_resume_settings

    assert_error "container 'gifted_mcnulty' is not a stopped claude sandbox, expected one of: sleepy_babbage",
                 sndbx('resume', 'claude', 'gifted_mcnulty')
    assert_empty docker_starts
  end

  def test_resume_without_stopped_containers
    write_exec_settings

    assert_error 'no stopped claude sandbox', sndbx('resume', 'claude', 'gifted_mcnulty')
  end

  def test_resume_unknown_image
    write_resume_settings

    assert_error "unknown image 'nope', expected one of: claude, codex", sndbx('resume', 'nope', 'sleepy_babbage')
  end

  def test_resume_needs_an_image_and_a_container
    write_resume_settings

    assert_usage_error "the following required arguments were not provided:\n  <IMAGE>", sndbx('resume')
    assert_usage_error "the following required arguments were not provided:\n  <CONTAINER>", sndbx('resume', 'claude')
    assert_usage_error "unexpected argument 'bash' found", sndbx('resume', 'claude', 'sleepy_babbage', 'bash')
  end

  def test_resume_without_docker
    write_resume_settings

    assert_error 'docker is not installed', sndbx('resume', 'claude', 'sleepy_babbage', docker: false)
  end
end
