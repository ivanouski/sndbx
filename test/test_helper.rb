# frozen_string_literal: true

require 'fileutils'
require 'json'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require 'yaml'

# Tests run in threads, so they keep to their own temporary root and don't change ENV or the working directory.
# MT_CPU=1 runs them one at a time.
Minitest::Test.parallelize_me!

# Lists the slowest tests after the summary, so a test that slows the suite down shows up in the next run
module SlowestTests
  COUNT = 5

  class Reporter < Minitest::AbstractReporter
    def initialize(io)
      super()
      @io = io
      @results = []
    end

    # Called from the test threads
    def record(result)
      @mutex.synchronize { @results << result }
    end

    def report
      return if @results.empty?

      @io.puts "\nSlowest tests:"
      @results.max_by(COUNT, &:time).each do |result|
        @io.puts format('  %<time>.2fs  %<test>s', time: result.time, test: result.location)
      end
    end
  end

  def self.minitest_plugin_init(options)
    Minitest.reporter << Reporter.new(options[:io])
  end
end
Minitest.register_plugin(SlowestTests)

module SndbxTestHelper
  # The binary to test, the debug build unless SNDBX_BIN names another, e.g. a release build
  COMMAND = [File.expand_path(ENV.fetch('SNDBX_BIN', '../target/debug/sndbx'), __dir__)].freeze

  # Unset, the host's would take sndbx's settings and log out of the temporary HOME
  XDG_ENV = { 'XDG_CONFIG_HOME' => nil, 'XDG_STATE_HOME' => nil }.freeze

  # Unset, the fake commands, Ruby scripts, would load bundler as bundle exec does, which takes most of their time
  BUNDLER_ENV = { 'RUBYOPT' => nil, 'BUNDLE_GEMFILE' => nil }.freeze

  Result = Struct.new(:out, :err, :status) do
    def success?
      status.success?
    end
  end

  def setup
    @root = File.realpath(Dir.mktmpdir)
    @home = mkdir('home')
    @bin = mkdir('bin')
    @work = mkdir('work/project')
    @images_dir = mkdir('images')
  end

  def teardown
    FileUtils.remove_entry(@root)
  end

  private

  # Without CLICOLOR_FORCE, which would color clap's errors
  def sndbx(*args, env: {})
    env = { 'HOME' => @home, 'PATH' => @bin, 'CLICOLOR_FORCE' => nil, **XDG_ENV, **BUNDLER_ENV }.merge(env)
    Result.new(*Open3.capture3(env, *COMMAND, *args, chdir: @work))
  end

  def mkdir(path)
    File.join(@root, path).tap { |dir| FileUtils.mkdir_p(dir) }
  end

  def write_executable(path, content)
    File.write(path, content)
    File.chmod(0o755, path)
  end

  def settings_path(home = @home)
    File.join(home, '.config', 'sndbx', 'settings.yml')
  end

  def write_settings(home: @home, **settings)
    write_settings_file(JSON.parse(settings.to_json).to_yaml, home: home)
  end

  def write_settings_file(content, home: @home)
    FileUtils.mkdir_p(File.dirname(settings_path(home)))
    File.write(settings_path(home), content)
  end

  def write_dockerfile(name, content)
    File.write(File.join(mkdir("images/#{name}"), 'Dockerfile'), content)
  end

  def write_images_settings
    write_settings(images: [{ name: 'claude', user: 'ubuntu' }, { name: 'codex', user: 'ubuntu' }])
  end

  # The fake docker's calls, logged to @docker_log
  def docker_calls
    File.readlines(@docker_log).map { |line| JSON.parse(line) }
  end

  # The options sndbx's help lists for the command
  def help_options(*command)
    sndbx(*command, '-h').out.lines
                         .map(&:strip)
                         .select { |line| line.start_with?('-') }
                         .flat_map { |line| line.split(/\s{2,}/).first.scan(/-[-\w]+/) }
                         .sort
  end
end
