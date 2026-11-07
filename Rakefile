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

desc 'Check sndbx, then run every test'
task check_and_test: %i[check test]

# RuboCop takes about as long as the rest, so it runs alongside
multitask default: %i[rubocop check_and_test]

LINUX_DIST = {
  'x86_64-unknown-linux-musl' => 'sndbx-linux-x86_64',
  'aarch64-unknown-linux-musl' => 'sndbx-linux-arm64'
}.freeze
MACOS_TARGETS = %w[x86_64-apple-darwin aarch64-apple-darwin].freeze
MACOS = RUBY_PLATFORM.include?('darwin')

def release_build(targets, env = {})
  sh 'rustup', 'target', 'add', *targets
  targets.each { |target| sh env, 'cargo', 'build', '--release', '--target', target }
  mkdir_p 'dist'
end

def release_binary(target)
  "target/#{target}/release/sndbx"
end

namespace :dist do
  desc 'Build static Linux binaries into dist/'
  task :linux do
    # Rust's bundled linker links musl binaries without a C cross toolchain
    env = LINUX_DIST.keys.to_h { |target| ["CARGO_TARGET_#{target.upcase.tr('-', '_')}_LINKER", 'rust-lld'] }
    release_build(LINUX_DIST.keys, env)
    LINUX_DIST.each { |target, asset| cp release_binary(target), "dist/#{asset}" }
  end

  desc 'Build a universal macOS binary into dist/ (on macOS only)'
  task :macos do
    abort 'dist:macos needs macOS for its SDK and lipo' unless MACOS
    release_build(MACOS_TARGETS)
    sh 'lipo', '-create', '-output', 'dist/sndbx-macos', *MACOS_TARGETS.map { |target| release_binary(target) }
  end
end

desc 'Build release binaries into dist/: Linux, and macOS when run on a Mac'
task dist: MACOS ? %w[dist:linux dist:macos] : %w[dist:linux] do
  puts 'Skipped dist:macos, it needs macOS' unless MACOS
end
