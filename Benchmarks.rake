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

  def run
    operations = positive_integer("FPS_OPERATIONS", 300)
    warmup = nonnegative_integer("FPS_WARMUP", 20)
    report_dir = ENV.fetch("FPS_REPORT_DIR", "target/benchmarks/fps")
    fixture = ENV["FPS_DOCUMENT"]
    FileUtils.mkdir_p(report_dir)

    Dir.mktmpdir("ascdraw-fps") do |temporary_dir|
      socket_path = File.join(temporary_dir, "control.sock")
      document_path = File.join(temporary_dir, "benchmark.toml")
      FileUtils.cp(fixture, document_path) if fixture
      log_path = File.join(report_dir, "ascdraw.log")

      File.open(log_path, "w") do |log|
        pid = Process.spawn(
          "target/release/ascdraw",
          "--automation-socket",
          socket_path,
          document_path,
          out: log,
          err: [:child, :out],
        )
        client = nil
        begin
          client = connect(socket_path, pid)
          reports = run_scenarios(client, warmup, operations)
          write_reports(report_dir, reports, warmup, operations, fixture)
          print_summary(reports, report_dir)
          client.request(command: "shutdown")
          client.close
          client = nil
          wait_for_exit(pid)
        ensure
          client&.close
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

  def run_scenarios(client, warmup, operations)
    reports = []
    reports << measure(client, "scroll", warmup, operations) do
      client.request(command: "scroll", x: 0.0, y: -1.0, steps: 1)
    end

    client.request(command: "key", key: "i", count: 1)
    character = false
    reports << measure(client, "text", warmup, operations) do
      character = !character
      client.request(command: "text", text: character ? "x" : " ")
    end

    client.request(command: "key", key: "escape", count: 1)
    client.request(command: "key", key: "1", count: 1)
    client.request(command: "key", key: "2", count: 1)
    reports << measure(client, "line", warmup, operations) do
      client.request(
        command: "key",
        key: "right",
        modifiers: { control: true },
        count: 1,
      )
    end
    reports
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
    puts format("%-10s %10s %12s %12s %10s %10s", "scenario", "FPS", "frame p95", "event p95", ">8.33ms", ">16.67ms")
    reports.each do |report|
      metrics = report.fetch("metrics")
      puts format(
        "%-10s %10.1f %10.2fms %10.2fms %9.1f%% %9.1f%%",
        report.fetch("scenario"),
        metrics.fetch("frame_rate"),
        metrics.dig("frames", "p95_ms"),
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
  desc "Run native-window scrolling, text, and line-drawing FPS benchmarks"
  task fps: :build_release do
    FpsBenchmark.run
  end
end
