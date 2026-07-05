require "fileutils"
require "shellwords"
require "tmpdir"

task :clean_app do
	sh "rm -fr /Applications/kakvide 2>/dev/null || true"
end

task :install_copy => [:clean_app] do
  sh "cp -pr ./target/release/bundle/osx/Kakvide.app /Applications"
end

task :install => [:clean_app, :bundle, :install_copy] do

end

task :bundle => [:build_release] do
	sh "cargo bundle --release --format osx"
end

task :build_release do
  sh "cargo build --release"
end

task :register do
  sh "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Kakvide.app"
end

task :icon do
  source = "assets/kakvide.png"
  output = ENV.fetch("ICON_OUTPUT", "target/generated/kakvide.icns")
  magick = `command -v magick`.strip

  abort "ImageMagick is required to build icons (`brew install imagemagick`)." if magick.empty?

  Dir.mktmpdir("kakvide-icon") do |dir|
    iconset = File.join(dir, "kakvide.iconset")
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
        source.shellescape,
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

    generated = File.join(dir, "kakvide.icns")
    body = chunks.join
    File.binwrite(generated, "icns" + [body.bytesize + 8].pack("N") + body)

    if File.exist?(output) && FileUtils.compare_file(generated, output)
      puts "#{output} is up to date"
    else
      FileUtils.mkdir_p(File.dirname(output))
      FileUtils.cp(generated, output)
    end
  end
end
