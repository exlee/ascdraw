module ScrollOffContentScenario
  module_function

  def run
    fixture = WorkspaceFixture.path(default: 2)
    log_dir = ENV.fetch("SCENARIO_LOG_DIR", "target/scenarios/scroll-off-content")
    FileUtils.mkdir_p(log_dir)

    Dir.mktmpdir("ascroll") do |temporary_dir|
      socket_path = File.join(temporary_dir, "control.sock")
      document_path = WorkspaceFixture.materialize(fixture, temporary_dir)

      File.open(File.join(log_dir, "ascdraw.log"), "w") do |log|
        FpsBenchmark.run_editor(socket_path, document_path, log) do |client|
          scroll(client, 500, "left")
          scroll(client, -1_000, "right")
          scroll(client, 500, "left")
        end
      end
    end
  end

  def scroll(client, distance, direction)
    puts "Scrolling #{distance.abs} #{direction}..."
    delta = distance.positive? ? 1.0 : -1.0
    distance.abs.times do
      client.request(command: "scroll", x: delta, y: 0.0, steps: 1)
    end
  end
end

namespace :scenario do
  namespace :scroll do
    desc "Load the fixture and smoothly scroll left off-content, right, then left"
    task "off-content": :build_release do
      ScrollOffContentScenario.run
    end
  end
end
