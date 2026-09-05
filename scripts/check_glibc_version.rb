#!/usr/bin/env ruby
# frozen_string_literal: true

# Fails when a built binary references a glibc symbol version newer than the
# allowed maximum. glibc is backwards compatible only, so a binary built on a
# newer distribution refuses to start on an older one.
#
# Usage: check_glibc_version.rb <binary> <max-version>

require "shellwords"

module GlibcVersionCheck
  module_function

  # Pulls "GLIBC_2.39"-style version tags out of `objdump -T` output.
  def required_versions(objdump_output)
    objdump_output.scan(/GLIBC_(\d+(?:\.\d+)*)/).flatten.uniq
  end

  def parse(version)
    version.split(".").map { |part| Integer(part, 10) }
  end

  def newer_than_max(versions, max_version)
    max = parse(max_version)
    versions.select { |version| (parse(version) <=> max) > 0 }
  end

  def sort_versions(versions)
    versions.sort_by { |version| parse(version) }
  end

  def read_symbols(binary)
    output = `objdump -T #{binary.shellescape} 2>/dev/null`
    return output if $?.success?

    warn "check_glibc_version: objdump failed on #{binary}"
    exit 2
  end
end

if $PROGRAM_NAME == __FILE__
  binary, max_version = ARGV
  if binary.nil? || max_version.nil?
    warn "usage: check_glibc_version.rb <binary> <max-version>"
    exit 2
  end

  unless File.file?(binary)
    warn "check_glibc_version: no such file: #{binary}"
    exit 2
  end

  versions = GlibcVersionCheck.required_versions(GlibcVersionCheck.read_symbols(binary))
  puts "Required glibc versions: #{GlibcVersionCheck.sort_versions(versions).join(', ')}"

  too_new = GlibcVersionCheck.newer_than_max(versions, max_version)
  unless too_new.empty?
    warn "check_glibc_version: #{binary} needs glibc " \
         "#{GlibcVersionCheck.sort_versions(too_new).join(', ')}, above the allowed #{max_version}"
    exit 1
  end

  puts "OK: nothing above glibc #{max_version}"
end
