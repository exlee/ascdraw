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

task :icon do
  source = "assets/kakvide.png"
  output = "assets/kakvide.icns"
  magick = `command -v magick`.strip

  abort "ImageMagick is required to build icons (`brew install imagemagick`)." if magick.empty?

  Dir.mktmpdir("kakvide-icon") do |dir|
    iconset = File.join(dir, "kakvide.iconset")
    FileUtils.mkdir_p(iconset)

    {
      "icon_16x16.png" => 16,
      "icon_16x16@2x.png" => 32,
      "icon_32x32.png" => 32,
      "icon_32x32@2x.png" => 64,
      "icon_128x128.png" => 128,
      "icon_128x128@2x.png" => 256,
      "icon_256x256.png" => 256,
      "icon_256x256@2x.png" => 512,
      "icon_512x512.png" => 512,
      "icon_512x512@2x.png" => 1024,
    }.each do |name, size|
      sharpness = size <= 32 ? "0x0.75+1.8+0.01" : "0x0.55+1.1+0.01"
      sh [
        magick.shellescape,
        source.shellescape,
        "-filter LanczosSharp",
        "-define filter:blur=0.70",
        "-resize #{size}x#{size}",
        "-unsharp #{sharpness}",
        File.join(iconset, name).shellescape,
      ].join(" ")
    end

    sh "iconutil --convert icns --output #{output} #{iconset}"
  end
end
