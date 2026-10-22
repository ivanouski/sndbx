# frozen_string_literal: true

require_relative 'test_helper'
require 'shellwords'

class ZshCompletionTest < Minitest::Test
  include SndbxTestHelper

  COMPLETION_DIR = File.expand_path('../completions/zsh', __dir__)

  COMMANDS = [[], ['run'], ['build'], ['rm'], ['exec'], ['resume'], ['list'], %w[list running], %w[list stopped],
              ['logs'], %w[logs commands], %w[logs proxy]].freeze

  # zsh with zpty, which types the line into an interactive zsh
  ZSH = system('zsh', '-fc', 'zmodload zsh/zpty', out: File::NULL, err: File::NULL)

  # compinit's dump with the completion, made once: compinit loads it in milliseconds, scanning fpath takes 100ms
  DUMP = if ZSH
           File.join(Dir.mktmpdir, 'zcompdump').tap do |dump|
             Minitest.after_run { FileUtils.remove_entry(File.dirname(dump)) }
             system({ 'COMPLETION_DIR' => COMPLETION_DIR, 'DUMP' => dump }, 'zsh', '-fc',
                    'fpath=($COMPLETION_DIR $fpath); autoload -Uz compinit; compinit -u -d $DUMP', exception: true)
           end
         end

  # Types the line and a tab into an interactive zsh, whose compadd records the matches in SNDBX_ZSH_OUT
  COMPLETE = <<~'ZSH'
    zmodload zsh/zpty zsh/zselect || exit 1
    zpty z zsh -f -i
    zpty -w z 'eval "$SNDBX_ZSH_SETUP"'
    zpty -w -n z "$1"$'\t'
    # Up to 20 seconds, in hundredths
    for (( n = 0; n < 2000; n++ )); do
      [[ -e $SNDBX_ZSH_DONE ]] && break
      zpty -rt z output
      zselect -t 1
    done
    zpty -d z
    [[ -e $SNDBX_ZSH_DONE ]] || { print -u2 'completion timed out'; exit 1 }
    exit 0
  ZSH

  # Loads the completion system from the dump, adds docker's completion when the test has one, and records the
  # matches compadd offers, except when it only collects them for its caller with -O, -A or -D
  SETUP = <<~'ZSH'
    exec 2>>$SNDBX_ZSH_ERR
    PS1= RPS1=
    fpath=($SNDBX_ZSH_COMPLETION_DIR $fpath)
    autoload -Uz compinit && compinit -C -d $SNDBX_ZSH_DUMP
    if [[ -n $SNDBX_ZSH_DOCKER_DIR ]]; then
      fpath=($SNDBX_ZSH_DOCKER_DIR $fpath)
      autoload -Uz _docker && compdef _docker docker
    fi
    zstyle ':completion:*' completer _complete
    compadd() {
      if [[ ${@[1,(i)(-|--)]} == *-(O|A|D)\ * ]]; then
        builtin compadd "$@"
        return
      fi
      local -a __sndbx_matches
      builtin compadd -A __sndbx_matches "$@"
      (( $#__sndbx_matches )) && print -rl -- $__sndbx_matches >> $SNDBX_ZSH_OUT
      builtin compadd "$@"
    }
    _sndbx_test_done() { : > $SNDBX_ZSH_DONE }
    comppostfuncs=(_sndbx_test_done)
  ZSH

  # Records its arguments and prints the containers in DOCKER_CONTAINERS
  FAKE_DOCKER = <<~RUBY
    #!#{RbConfig.ruby} --disable-gems
    require 'json'

    File.write(ENV.fetch('DOCKER_LOG'), "\#{ARGV.to_json}\\n", mode: 'a')
    containers = ENV.fetch('DOCKER_CONTAINERS')
    print(File.read(containers)) if ARGV.first == 'ps' && File.exist?(containers)
  RUBY

  # Docker's zsh completion, which records the words it completes and offers two options
  FAKE_DOCKER_COMPLETION = <<~ZSH
    #compdef docker
    print -r -- "$CURRENT $words" >> $DOCKER_COMPLETION_LOG
    compadd -- --memory --memory-swap
  ZSH

  def setup
    skip 'zsh with zpty is not installed' unless ZSH
    super
    @docker_log = File.join(@root, 'docker.log')
    @docker_containers = File.join(@root, 'docker-containers')
    @docker_completion_log = File.join(@root, 'docker-completion.log')

    write_executable(File.join(@bin, 'sndbx'), "#!/bin/sh\nexec #{Shellwords.join(COMMAND)} \"$@\"\n")
    write_executable(File.join(@bin, 'docker'), FAKE_DOCKER)
  end

  def teardown
    super if ZSH
  end

  def test_options_match_help
    COMMANDS.each do |command|
      assert_equal help_options(*command), complete([*command, '-'].join(' ')), "options of sndbx #{command.join(' ')}"
    end
  end

  # Each description is the start of the help's, so a changed help fails here until the completion follows
  def test_descriptions_match_help
    help = help_descriptions
    descriptions = completion_descriptions

    assert_equal help.keys.sort, descriptions.map(&:first).uniq.sort
    descriptions.each do |name, description|
      refute_empty description, name
      assert help.fetch(name).start_with?(description), "#{name}: #{description} doesn't start #{help.fetch(name)}"
    end
  end

  def test_commands
    assert_equal %w[build exec list logs resume rm run], complete('')
    assert_equal %w[resume rm run], complete('r')
    assert_equal %w[run], complete('ru')
  end

  def test_unknown_command
    assert_empty complete('foo ')
  end

  def test_run_images
    write_images_settings

    assert_equal %w[claude codex], complete('run ')
    assert_equal %w[codex], complete('run co')
  end

  def test_run_images_after_option_values
    write_images_settings
    api = mkdir('work/api')

    assert_equal %w[claude codex], complete("run -v #{api} -w api -p 3000 ")
    assert_equal %w[claude codex], complete("run --volume=#{api} --publish=3000 ")
    assert_equal %w[claude codex], complete("run -v #{api}:/opt/api:ro -p 8080:80 ")
  end

  def test_volume_target_completes_nothing
    mkdir('work/project/src')

    assert_empty complete('run -v src:')
    assert_empty complete('run --volume=src:')
    assert_empty complete('run -v src:/opt:')
  end

  def test_docker_args_complete_with_docker
    write_docker_completion

    assert_equal %w[--memory --memory-swap], complete('run claude -v . -- --init --mem')
    assert_equal ['4 docker run --init --mem'], File.readlines(@docker_completion_log, chomp: true)
  end

  def test_docker_args_complete_files_without_docker_completion
    File.write(File.join(@work, 'env'), '')
    mkdir('work/project/src')

    assert_equal %w[env src], complete('run claude -- --env-file ')
  end

  def test_run_only_options_after_image
    write_images_settings

    assert_equal help_options('run'), complete('run claude ')
  end

  def test_run_images_without_settings_do_not_create_them
    assert_empty complete('run ')
    refute File.exist?(settings_path)
  end

  def test_build_images
    write_dockerfile('base', "FROM ubuntu\n")
    write_dockerfile('claude', "FROM ubuntu\n")
    write_dockerfile('codex', "FROM ubuntu\n")
    write_settings(images_dir: @images_dir, images: [])

    assert_equal %w[claude codex], complete('build ')
    assert_equal %w[codex], complete('build claude ')
    assert_equal %w[claude], complete('build --rebuild-base codex ')
    assert_empty complete('build claude codex ')
    assert_equal %w[--rebuild-base], complete('build --r')
  end

  def test_rm_images
    write_images_settings

    assert_equal %w[claude codex], complete('rm ')
    assert_equal %w[codex], complete('rm claude ')
    assert_empty complete('rm claude codex ')
  end

  def test_exec_images
    write_images_settings

    assert_equal %w[claude codex], complete('exec ')
    assert_equal %w[claude codex], complete('exec -u root ')
    assert_equal %w[claude codex], complete('exec --user=root ')
  end

  def test_exec_containers
    write_images_settings
    File.write(@docker_containers, "gifted_mcnulty\nquirky_hopper\n")

    assert_equal %w[gifted_mcnulty quirky_hopper], complete('exec claude ')
    assert_equal %w[quirky_hopper], complete('exec -u root claude q')
    assert_equal [['info'], ['ps', '--filter', 'label=sndbx.image=claude', '--format', '{{.Names}}']],
                 docker_calls.last(2)
  end

  def test_exec_options_before_the_command
    assert_equal %w[--user], complete('exec claude gifted_mcnulty --u')
  end

  def test_exec_completes_nothing_from_the_command_on
    File.write(File.join(@work, 'README.md'), '')

    assert_empty complete('exec claude gifted_mcnulty ')
    assert_empty complete('exec claude gifted_mcnulty ls ')
    assert_empty complete('exec claude gifted_mcnulty ls -')
    assert_empty complete('exec claude gifted_mcnulty -- -')
    assert_empty complete('exec -u ')
  end

  def test_resume_images
    write_images_settings

    assert_equal %w[claude codex], complete('resume ')
  end

  def test_resume_containers
    write_images_settings
    File.write(@docker_containers, "sleepy_babbage\nzen_lovelace\n")

    assert_equal %w[sleepy_babbage zen_lovelace], complete('resume claude ')
    assert_equal %w[zen_lovelace], complete('resume claude z')
    assert_equal [['info'],
                  ['ps', '--all', '--filter', 'status=exited', '--filter', 'label=sndbx.image=claude',
                   '--format', '{{.Names}}']],
                 docker_calls.last(2)
  end

  def test_resume_completes_only_options_after_the_container
    write_images_settings

    assert_equal %w[--help -h], complete('resume claude sleepy_babbage ')
  end

  def test_list_what
    assert_equal %w[buildable configured running stopped], complete('list ')
    assert_equal %w[running], complete('list r')
  end

  def test_list_images_for_running_and_stopped
    write_images_settings

    assert_equal %w[claude codex], complete('list running ')
    assert_equal %w[codex], complete('list stopped co')
  end

  def test_list_completes_only_options_after_the_last_argument
    write_images_settings

    assert_equal %w[--help -h], complete('list configured ')
    assert_equal %w[--help -h], complete('list buildable ')
    assert_equal %w[--help -h], complete('list running claude ')
  end

  def test_logs_completes_what_to_follow
    assert_equal %w[commands proxy], complete('logs ')
    assert_equal %w[proxy], complete('logs p')
  end

  def test_logs_completes_only_options_after_what
    assert_equal help_options('logs', 'commands'), complete('logs commands ')
    assert_equal %w[--denied --help -d -h], complete('logs proxy -n 5 ')
  end

  def test_logs_completes_no_line_count
    assert_empty complete('logs proxy -n ')
    assert_empty complete('logs proxy -n -')
  end

  def test_volume_completes_directories
    mkdir('work/project/src')
    File.write(File.join(@work, 'README.md'), '')

    assert_equal %w[src], complete('run -v ')
  end

  def test_publish_completes_nothing
    assert_empty complete('run -p ')
  end

  def test_workdir_defaults_to_the_current_directory
    assert_equal %w[project], complete('run claude -w ')
  end

  def test_workdir_completes_volumes
    api = mkdir('work/api')
    mkdir('work/web')
    mkdir('home/notes')

    assert_equal %w[api notes web], complete("run -v #{api} -v ../web --volume ~/notes -w ")
  end

  def test_workdir_completes_volume_targets
    api = mkdir('work/api')
    mkdir('work/sdk')
    File.write(File.join(@work, 'notes.txt'), '')

    assert_equal %w[/opt/sdk project], complete('run -v ../sdk:/opt/sdk:ro -v notes.txt:/notes.txt -w ')
    assert_equal %w[/opt/sdk backend], complete(%(run -v "../sdk:/opt/sdk" -v#{api}:/workspace/backend -w ))
  end

  def test_workdir_completes_the_workspace_root
    api = mkdir('work/api')

    assert_equal %w[/workspace], complete("run -v #{api}:/workspace -w ")
  end

  def test_workdir_skips_missing_volumes
    api = mkdir('work/api')

    assert_equal %w[api], complete("run -v #{api} -v #{File.join(@root, 'missing')} -w ")
  end

  def test_workdir_completes_grouped_and_attached_volumes
    api = mkdir('work/api')
    mkdir('work/web')
    mkdir('work/sdk')

    assert_equal %w[api sdk web], complete("run -kv #{api} -v../web -v=../sdk -w ")
    assert_equal %w[project], complete('run claude -kw ')
  end

  def test_grouped_volume_completes_directories
    mkdir('work/project/src')
    File.write(File.join(@work, 'README.md'), '')

    assert_equal %w[src], complete('run -kv ')
  end

  def test_workdir_with_equals_sign
    api = mkdir('work/api')

    assert_equal %w[api], complete("run --volume=#{api} --workdir=")
  end

  private

  # The matches for `sndbx <line>`, completed at its end
  def complete(line)
    out_path = File.join(@root, 'zsh-out')
    err_path = File.join(@root, 'zsh-err')
    done_path = File.join(@root, 'zsh-done')
    [out_path, err_path, done_path].each { |path| FileUtils.rm_f(path) }
    env = { 'HOME' => @home, 'PATH' => "#{@bin}:#{ENV.fetch('PATH')}", 'SNDBX_ZSH_SETUP' => SETUP,
            'SNDBX_ZSH_COMPLETION_DIR' => COMPLETION_DIR, 'SNDBX_ZSH_DUMP' => DUMP,
            'SNDBX_ZSH_DOCKER_DIR' => @docker_completion_dir, 'SNDBX_ZSH_OUT' => out_path, 'SNDBX_ZSH_ERR' => err_path,
            'SNDBX_ZSH_DONE' => done_path, 'DOCKER_LOG' => @docker_log, 'DOCKER_CONTAINERS' => @docker_containers,
            'DOCKER_COMPLETION_LOG' => @docker_completion_log, **XDG_ENV, **BUNDLER_ENV }
    _out, err, status = Open3.capture3(env, 'zsh', '-f', '-c', COMPLETE, 'zsh', "sndbx #{line}", chdir: @work)
    assert_predicate status, :success?, err
    assert_empty File.exist?(err_path) ? File.read(err_path) : '', "errors completing sndbx #{line}"

    File.exist?(out_path) ? File.readlines(out_path, chomp: true).uniq.sort : []
  end

  def write_docker_completion
    @docker_completion_dir = mkdir('zsh-functions')
    File.write(File.join(@docker_completion_dir, '_docker'), FAKE_DOCKER_COMPLETION)
  end

  # Option and command names to their descriptions in sndbx's help
  def help_descriptions
    COMMANDS.each_with_object({}) do |command, descriptions|
      section = nil
      sndbx(*command, '-h').out.each_line(chomp: true) do |line|
        section = line unless line.start_with?(' ')
        next unless line.start_with?('  ') && ['Options:', 'Commands:'].include?(section)

        names, description = line.strip.split(/\s{2,}/, 2)
        names.scan(/-[-\w]+|\A\w+\z/).each { |name| descriptions[name] = description }
      end
    end
  end

  # Option and command names with their descriptions in the completion, like {-v+,--volume=}'[Mount ...]'
  # and 'run:Run a sandboxed image'
  def completion_descriptions
    content = File.read(File.join(COMPLETION_DIR, '_sndbx'))
    options = content.scan(/\{(-\w)\+?,(--[\w-]+)=?\}['"]\[([^\]]*)\]/).flat_map do |short, long, description|
      [[short, description], [long, description]]
    end
    options + content.scan(/^\s+(['"])([a-z]+):(.+)\1$/).map { |_quote, name, description| [name, description] }
  end
end
