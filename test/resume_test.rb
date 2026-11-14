# frozen_string_literal: true

require_relative 'sndbx_test_case'

class ResumeTest < SndbxTestCase
  def test_resume
    write_resume_settings

    sndbx('resume', 'sleepy_babbage')

    assert_equal [['info'],
                  ['ps', '--all', '--filter', 'status=exited', '--filter', 'label=sndbx.image',
                   '--format', '{{.Names}}'],
                  ['start', '-ai', 'sleepy_babbage']],
                 docker_calls
  end

  def test_resume_sandbox_of_any_image
    write_resume_settings
    add_docker_container('codex', 'zen_lovelace', 'exited')

    sndbx('resume', 'zen_lovelace')

    assert_equal [['start', '-ai', 'zen_lovelace']], docker_starts
  end

  def test_resume_running_container
    write_resume_settings

    assert_error "container 'gifted_mcnulty' is not a stopped sandbox, expected one of: sleepy_babbage",
                 sndbx('resume', 'gifted_mcnulty')
    assert_empty docker_starts
  end

  def test_resume_without_stopped_containers
    write_exec_settings

    assert_error 'no stopped sandbox', sndbx('resume', 'gifted_mcnulty')
  end
end
