require "fileutils"
require "shellwords"
require "tmpdir"

ICON_SOURCE = "assets/ascdraw.png"
ICON_OUTPUT = "target/generated/ascdraw.icns"

task :clean_app do
  sh "rm -fr /Applications/Ascdraw.app 2>/dev/null || true"
end

task :install_copy => [:clean_app] do
  sh "cp -pr ./target/release/bundle/osx/Ascdraw.app /Applications"
end

task :install => [:clean_app, :bundle, :install_copy, :register]

task :bundle => [ICON_OUTPUT, :build_release] do
  sh "cargo bundle --release --format osx"
end

desc "Build release binaries"
task :build_release do
  sh "cargo build --release --bins --locked"
end

task :register => [:unregister_target] do
  sh "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Ascdraw.app"
end

task :unregister_target do
  sh "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u target/release/bundle/osx/Ascdraw.app 2>/dev/null || true"
end

task :icon => [ICON_OUTPUT]

file ICON_OUTPUT => [ICON_SOURCE] do
  magick = `command -v magick`.strip

  abort "ImageMagick is required to build icons (`brew install imagemagick`)." if magick.empty?

  Dir.mktmpdir("ascdraw-icon") do |dir|
    iconset = File.join(dir, "ascdraw.iconset")
    FileUtils.mkdir_p(iconset)

    icons = [
      ["icp4", "icon_16x16.png", 16],
      ["icp5", "icon_32x32.png", 32],
      ["icp6", "icon_32x32@2x.png", 64],
      ["ic07", "icon_128x128.png", 128],
      ["ic08", "icon_256x256.png", 256],
      ["ic09", "icon_512x512.png", 512],
      ["ic10", "icon_512x512@2x.png", 1024],
    ]

    chunks = icons.map do |icon_type, name, size|
      path = File.join(iconset, name)
      sharpness = size <= 32 ? "0x0.75+1.8+0.01" : "0x0.55+1.1+0.01"
      sh [
        magick.shellescape,
        ICON_SOURCE.shellescape,
        "-filter LanczosSharp",
        "-define filter:blur=0.70",
        "-resize #{size}x#{size}",
        "-unsharp #{sharpness}",
        "-strip",
        "-define png:exclude-chunk=all",
        path.shellescape,
      ].join(" ")

      png = File.binread(path)
      icon_type + [png.bytesize + 8].pack("N") + png
    end

    generated = File.join(dir, "ascdraw.icns")
    body = chunks.join
    File.binwrite(generated, "icns" + [body.bytesize + 8].pack("N") + body)

    if File.exist?(ICON_OUTPUT) && FileUtils.compare_file(generated, ICON_OUTPUT)
      puts "#{ICON_OUTPUT} is up to date"
    else
      FileUtils.mkdir_p(File.dirname(ICON_OUTPUT))
      FileUtils.cp(generated, ICON_OUTPUT)
    end
  end
end

import "Benchmarks.rake"
