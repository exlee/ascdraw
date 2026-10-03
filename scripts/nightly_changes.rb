#!/usr/bin/env ruby
# frozen_string_literal: true

# Decides whether the nightly workflow should build. A scheduled run builds
# only when the commit differs from the one the latest nightly release was
# tagged at; a manual run always builds.
#
# Usage: nightly_changes.rb <event-name> <head-sha>
# Needs GITHUB_REPOSITORY and a GH_TOKEN for the `gh` CLI. Writes
# `build=true|false` to $GITHUB_OUTPUT when set.

require "json"
require "open3"

module NightlyChanges
  module_function

  TAG_PREFIX = "nightly-"

  # Picks the newest published nightly release from the GitHub releases API
  # response. Drafts never got a tag pushed, so they do not count.
  def latest_nightly_tag(releases)
    nightly = releases.select do |release|
      !release["draft"] && release["tag_name"].to_s.start_with?(TAG_PREFIX)
    end
    nightly.max_by { |release| release["created_at"].to_s }&.fetch("tag_name")
  end

  def should_build?(event:, head_sha:, last_sha:)
    return true if event == "workflow_dispatch"
    return true if last_sha.nil? || last_sha.empty?

    head_sha != last_sha
  end

  def gh_api(path, *args)
    output, status = Open3.capture2("gh", "api", path, *args)
    return output if status.success?

    warn "nightly_changes: gh api #{path} failed"
    exit 2
  end

  def fetch_releases(repo)
    JSON.parse(gh_api("repos/#{repo}/releases?per_page=100"))
  end

  def tag_commit(repo, tag)
    gh_api("repos/#{repo}/commits/#{tag}", "--jq", ".sha").strip
  end
end

if $PROGRAM_NAME == __FILE__
  event, head_sha = ARGV
  repo = ENV.fetch("GITHUB_REPOSITORY", nil)
  if event.nil? || head_sha.nil? || repo.nil?
    warn "usage: GITHUB_REPOSITORY=<owner/repo> nightly_changes.rb <event-name> <head-sha>"
    exit 2
  end

  tag = NightlyChanges.latest_nightly_tag(NightlyChanges.fetch_releases(repo))
  last_sha = tag && NightlyChanges.tag_commit(repo, tag)
  puts "Latest nightly: #{tag || 'none'} at #{last_sha || 'n/a'}"
  puts "Head: #{head_sha} (#{event})"

  build = NightlyChanges.should_build?(event: event, head_sha: head_sha, last_sha: last_sha)
  puts build ? "Building nightly" : "No changes since #{tag}; skipping"

  if (output = ENV["GITHUB_OUTPUT"])
    File.open(output, "a") { |file| file.puts "build=#{build}" }
  end
end
