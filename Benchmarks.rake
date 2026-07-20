require "json"
require "socket"
require "timeout"
require "time"

class FpsBenchmarkClient
  def initialize(socket_path)
    @socket = UNIXSocket.new(socket_path)
    @next_id = 0
  end

  def request(command)
    @next_id += 1
    @socket.puts(JSON.generate({ id: @next_id }.merge(command)))
    ready = IO.select([@socket], nil, nil, 30)
    raise "ascdraw did not respond within 30 seconds" unless ready

    line = @socket.gets
    raise "ascdraw closed the automation connection" unless line

    response = JSON.parse(line)
    unless response["id"] == @next_id
      raise "automation response ID #{response["id"]} did not match #{@next_id}"
    end
    raise response.fetch("error", "automation command failed") unless response["ok"]

    response["result"]
  end

  def close
    @socket.close
  end
end

module FpsBenchmark
  module_function

  def point_on_circle(t, radius)
    angle = 2.0 * Math::PI * t

    x = radius * Math.cos(angle)
    y = radius * Math.sin(angle)

    [x, y]
  end

  def run
    operations = positive_integer("FPS_OPERATIONS", 300)
    warmup = nonnegative_integer("FPS_WARMUP", 20)
    report_dir = ENV.fetch("FPS_REPORT_DIR", "target/benchmarks/fps")
    fixture = WorkspaceFixture.path(default: 1)
    FileUtils.mkdir_p(report_dir)

    Dir.mktmpdir("ascdraw-fps") do |temporary_dir|
      socket_path = File.join(temporary_dir, "control.sock")
      document_path = WorkspaceFixture.materialize(fixture, temporary_dir)
      saved_document_path = WorkspaceFixture.materialize(
        fixture,
        temporary_dir,
        basename: "saved-workspace",
      )
      log_path = File.join(report_dir, "ascdraw.log")

      File.open(log_path, "w") do |log|
        reports = run_editor(socket_path, document_path, log) do |client|
          run_scenarios(client, warmup, operations) +
            [measure_zoom(client, "zoom-file", warmup, operations)]
        end
        reports.concat(
          run_editor(socket_path, saved_document_path, log) do |client|
            run_scenarios(client, warmup, operations, "saved-file-")
          end,
        )
        empty_home = File.join(temporary_dir, "empty-home")
        FileUtils.mkdir_p(empty_home)
        reports << run_editor(socket_path, nil, log, { "HOME" => empty_home }) do |client|
          measure_zoom(client, "zoom-empty", warmup, operations)
        end
        write_reports(report_dir, reports, warmup, operations, fixture)
        print_summary(reports, report_dir)
      end
    end
  end

  def run_mini
    operations = positive_integer("FPS_MINI_OPERATIONS", 30)
    warmup = nonnegative_integer("FPS_MINI_WARMUP", 0)
    report_dir = ENV.fetch("FPS_MINI_REPORT_DIR", "target/benchmarks/fps-mini")
    fixture = WorkspaceFixture.path(default: 1)
    FileUtils.mkdir_p(report_dir)

    Dir.mktmpdir("ascdraw-fps-mini") do |temporary_dir|
      socket_path = File.join(temporary_dir, "control.sock")
      document_path = WorkspaceFixture.materialize(fixture, temporary_dir)
      log_path = File.join(report_dir, "ascdraw.log")

      File.open(log_path, "w") do |log|
        reports = run_editor(socket_path, document_path, log) do |client|
          run_scenarios(client, warmup, operations, include_scroll: false)
        end
        write_reports(report_dir, reports, warmup, operations, fixture)
        print_summary(reports, report_dir)
      end
    end
  end

  def run_zoom
    operations = positive_integer("ZOOM_OPERATIONS", 1_200)
    warmup = nonnegative_integer("ZOOM_WARMUP", 0)
    report_dir = ENV.fetch("ZOOM_REPORT_DIR", "target/benchmarks/zoom")
    fixture = WorkspaceFixture.path(default: 1)
    FileUtils.mkdir_p(report_dir)

    Dir.mktmpdir("ascdraw-zoom") do |temporary_dir|
      socket_path = File.join(temporary_dir, "control.sock")
      document_path = WorkspaceFixture.materialize(fixture, temporary_dir)
      log_path = File.join(report_dir, "ascdraw.log")

      File.open(log_path, "w") do |log|
        report = run_editor(socket_path, document_path, log) do |client|
          measure_zoom(client, "zoom-file", warmup, operations)
        end
        reports = [report]
        write_reports(report_dir, reports, warmup, operations, fixture)
        print_summary(reports, report_dir)
      end
    end
  end

  def run_editor(socket_path, document_path, log, environment = {})
    arguments = [
      "target/release/ascdraw",
      "--automation-socket",
      socket_path,
    ]
    arguments << document_path if document_path
    pid = Process.spawn(
      environment,
      *arguments,
      out: log,
      err: [:child, :out],
    )
    client = nil
    begin
      client = connect(socket_path, pid)
      yield client
    ensure
      begin
        client&.request(command: "shutdown")
      ensure
        client&.close
        begin
          wait_for_exit(pid)
        ensure
          stop_process(pid)
        end
      end
    end
  end

  def connect(socket_path, pid)
    deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + 15
    loop do
      if File.socket?(socket_path)
        client = FpsBenchmarkClient.new(socket_path)
        result = client.request(command: "ping")
        return client if result["ready"]
        client.close
      end
      if Process.waitpid(pid, Process::WNOHANG)
        raise "ascdraw exited before the automation socket became ready"
      end
      raise "ascdraw did not become ready within 15 seconds" if Process.clock_gettime(Process::CLOCK_MONOTONIC) >= deadline

      sleep 0.05
    end
  end

  def run_scenarios(client, warmup, operations, name_prefix = "", include_scroll: true)
    reports = []
    if include_scroll
      circle_points = 100
      circle_radius = 40.0
      scroll_step = 0
      previous_point = point_on_circle(0.0, circle_radius)
      reports << measure(client, "#{name_prefix}scroll", warmup, operations) do
        t = (scroll_step % circle_points).fdiv(circle_points - 1)
        point = point_on_circle(t, circle_radius)
        scroll_step += 1
        x = point[0] - previous_point[0]
        y = point[1] - previous_point[1]
        previous_point = point
        client.request(command: "scroll", x: x, y: y, steps: 1)
      end
    end

    client.request(command: "key", key: "i", count: 1)
    characters = "□■▫▪◆◊·∙•●△▽◁▷▲▼◀▶↑↓░▒▓█▘▝▖▗▌▐▞▚▛▜α×▪+ø◦◯Øø╳╱╲÷×±←→▵▿◃▹▴▾◂▸▙▟▀▄█β▪↓+ø¤◇☆★※↕↔▏▎▍▋▊▉▔δ▪↑↓+ø▁▂▃▅▆▇▕γ▪↑+ø".chars
    character_index = 0
    inserts = 0
    reports << measure(client, "#{name_prefix}text", warmup, operations) do
      client.request(command: "text", text: characters[character_index])
      character_index = (character_index + 1) % characters.length
      inserts += 1
      if (inserts % 10).zero?
        client.request(command: "key", key: "left", count: 10)
        client.request(command: "key", key: "down", count: 1)
      end
    end

    client.request(command: "key", key: "escape", count: 1)
    client.request(command: "key", key: "1", count: 1)
    client.request(command: "key", key: "2", count: 1)
    spiral_directions = ["right", "up", "left", "down"]
    direction_index = 0
    segment_length = 1
    steps_in_segment = 0
    reports << measure(client, "#{name_prefix}line", warmup, operations) do
      client.request(
        command: "key",
        key: spiral_directions[direction_index],
        modifiers: { control: true },
        count: 1,
      )
      steps_in_segment += 1
      if steps_in_segment == segment_length
        steps_in_segment = 0
        direction_index = (direction_index + 1) % spiral_directions.length
        segment_length += 1
      end
    end
    reports
  end

  def measure_zoom(client, name, warmup, operations)
    minimum = 5.0
    maximum = 48.0
    step = 0.25
    current = minimum
    direction = 1.0

    client.request(command: "zoom", delta: -1_000.0)
    client.request(command: "zoom", delta: minimum - 4.0)
    measure(client, name, warmup, operations) do
      client.request(command: "zoom", delta: step * direction)
      current += step * direction
      direction = -1.0 if current >= maximum
      direction = 1.0 if current <= minimum
    end
  end

  def measure(client, name, warmup, operations, &operation)
    warmup.times(&operation)
    client.request(command: "metrics", reset: true)
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    operations.times(&operation)
    elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
    metrics = client.request(command: "metrics", reset: false)
    {
      "scenario" => name,
      "operations" => operations,
      "elapsed_seconds" => elapsed,
      "operations_per_second" => operations / elapsed,
      "metrics" => metrics,
    }
  end

  def write_reports(report_dir, reports, warmup, operations, fixture)
    reports.each do |report|
      path = File.join(report_dir, "#{report.fetch("scenario")}.json")
      File.write(path, JSON.pretty_generate(report) + "\n")
    end
    summary = {
      "generated_at" => Time.now.utc.iso8601,
      "build" => "release",
      "warmup_operations" => warmup,
      "measured_operations" => operations,
      "fixture" => fixture,
      "scenarios" => reports,
    }
    File.write(File.join(report_dir, "summary.json"), JSON.pretty_generate(summary) + "\n")
  end

  def print_summary(reports, report_dir)
    puts
    puts "FPS benchmark"
    puts format("%-18s %10s %12s %12s %12s %12s %10s %10s", "scenario", "FPS", "frame p95", "grid p95", "minimap p95", "event p95", ">8.33ms", ">16.67ms")
    reports.each do |report|
      metrics = report.fetch("metrics")
      puts format(
        "%-18s %10.1f %10.2fms %10.2fms %10.2fms %10.2fms %9.1f%% %9.1f%%",
        report.fetch("scenario"),
        metrics.fetch("frame_rate"),
        metrics.dig("frames", "p95_ms"),
        metrics.dig("grid", "p95_ms"),
        metrics.dig("minimap", "p95_ms"),
        metrics.dig("event_to_submit", "p95_ms"),
        metrics.fetch("over_8_33_ms_percent"),
        metrics.fetch("over_16_67_ms_percent"),
      )
    end
    puts "Reports: #{report_dir}"
  end

  def wait_for_exit(pid)
    Timeout.timeout(10) { Process.wait(pid) }
  rescue Timeout::Error
    raise "ascdraw did not exit within 10 seconds"
  end

  def stop_process(pid)
    Process.kill("TERM", pid)
    Process.wait(pid)
  rescue Errno::ESRCH, Errno::ECHILD
    nil
  end

  def positive_integer(name, default)
    value = Integer(ENV.fetch(name, default.to_s), 10)
    raise "#{name} must be greater than zero" unless value.positive?

    value
  rescue ArgumentError
    raise "#{name} must be an integer"
  end

  def nonnegative_integer(name, default)
    value = Integer(ENV.fetch(name, default.to_s), 10)
    raise "#{name} must not be negative" if value.negative?

    value
  rescue ArgumentError
    raise "#{name} must be an integer"
  end
end

namespace :benchmark do
  desc "Run native-window FPS benchmarks before and after saving and reopening the active file"
  task fps: :build_release do
    FpsBenchmark.run
  end

  desc "Run a short native-window text and line FPS benchmark"
  task fps_mini: :build_release do
    FpsBenchmark.run_mini
  end

  desc "Run a fixture-backed native-window zoom benchmark"
  task zoom: :build_release do
    FpsBenchmark.run_zoom
  end
end
