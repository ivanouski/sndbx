# frozen_string_literal: true

require 'fileutils'
require 'json'
require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require 'yaml'

module SndbxTestHelper
  # The binary to test, the debug build unless SNDBX_BIN names another, e.g. a release build
  COMMAND = [File.expand_path(ENV.fetch('SNDBX_BIN', '../target/debug/sndbx'), __dir__)].freeze

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
    env = { 'HOME' => @home, 'PATH' => @bin, 'CLICOLOR_FORCE' => nil }.merge(env)
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
end
