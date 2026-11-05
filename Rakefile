# frozen_string_literal: true

require 'rake/testtask'
require 'rubocop/rake_task'

desc 'Build sndbx and sndbx-top'
task :build do
  sh 'cargo', 'build', '--workspace'
end

desc 'Check sndbx and sndbx-top with rustfmt, clippy and their unit tests'
task :check do
  sh 'cargo', 'fmt', '--all', '--check'
  sh 'cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'
  sh 'cargo', 'test', '--workspace'
end

Rake::TestTask.new do |t|
  t.pattern = 'test/*_test.rb'
end
task test: :build

RuboCop::RakeTask.new

desc 'Check sndbx, then run every test'
task check_and_test: %i[check test]

# RuboCop takes about as long as the rest, so it runs alongside
multitask default: %i[rubocop check_and_test]
