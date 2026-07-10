task :clean_app do
  sh "rm -fr /Applications/ascdraw.app 2>/dev/null || true"
end

task :install_copy => [:clean_app] do
  sh "cp -pr ./target/release/bundle/osx/ascdraw.app /Applications"
end

task :install => [:clean_app, :bundle, :install_copy, :register]

task :bundle => [:build_release] do
  sh "cargo bundle --release --format osx"
end

task :build_release do
  sh "cargo build --release"
end

task :register => [:unregister_target] do
  sh "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/ascdraw.app"
end

task :unregister_target do
  sh "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u target/release/bundle/osx/ascdraw.app 2>/dev/null || true"
end
