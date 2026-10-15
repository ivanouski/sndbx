# frozen_string_literal: true

require 'rake/testtask'
require 'rubocop/rake_task'

desc 'Build sndbx'
task :build do
  sh 'cargo', 'build'
end

desc 'Check sndbx with rustfmt, clippy and its unit tests'
task :check do
  sh 'cargo', 'fmt', '--check'
  sh 'cargo', 'clippy', '--all-targets', '--', '-D', 'warnings'
  sh 'cargo', 'test'
end

Rake::TestTask.new do |t|
  t.pattern = 'test/*_test.rb'
end
task test: :build

RuboCop::RakeTask.new

task default: %i[check rubocop test]
