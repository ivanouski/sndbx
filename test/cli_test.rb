# frozen_string_literal: true

require_relative 'sndbx_test_case'

class CliTest < SndbxTestCase
  def test_no_arguments_prints_help
    assert_help 'Usage: sndbx command [options]', sndbx
  end

  def test_help
    assert_help 'Usage: sndbx command [options]', sndbx('-h')
  end

  def test_version
    result = sndbx('-V')

    assert_predicate result, :success?
    assert_match(/\Asndbx \d+\.\d+\.\d+\n\z/, result.out)
  end

  def test_missing_home
    missing = File.join(@root, 'missing')

    assert_predicate sndbx('-h', env: { 'HOME' => missing }), :success?
    assert_predicate sndbx('list', 'configured', env: { 'HOME' => missing }), :success?
  end
end
