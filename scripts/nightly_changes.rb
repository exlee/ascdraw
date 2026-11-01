#!/usr/bin/env ruby
# frozen_string_literal: true

# Decides whether the nightly Buildkite pipeline should build. A scheduled
# build runs only when the commit differs from the one the latest nightly
# release was tagged at; any other build source always builds.
#
# Usage: nightly_changes.rb <build-source> <head-sha> <pipeline-file>
# Needs GH_TOKEN for the `gh` CLI. `gh` resolves the repository from the
# checkout's origin remote. When a build is due, uploads <pipeline-file>
# with `buildkite-agent pipeline upload`.

require "json"
require "open3"

module NightlyChanges
  module_function

  TAG_PREFIX = "nightly-"
  SCHEDULE_SOURCE = "schedule"
  REPO_PATH = "repos/{owner}/{repo}"

  # Picks the newest published nightly release from the GitHub releases API
  # response. Drafts never got a tag pushed, so they do not count.
  def latest_nightly_tag(releases)
    nightly = releases.select do |release|
      !release["draft"] && release["tag_name"].to_s.start_with?(TAG_PREFIX)
    end
    nightly.max_by { |release| release["created_at"].to_s }&.fetch("tag_name")
  end

  def should_build?(source:, head_sha:, last_sha:)
    return true unless source == SCHEDULE_SOURCE
    return true if last_sha.nil? || last_sha.empty?

    head_sha != last_sha
  end

  def run!(*command)
    output, status = Open3.capture2(*command)
    return output if status.success?

    warn "nightly_changes: #{command.join(' ')} failed"
    exit 2
  end

  def fetch_releases
    JSON.parse(run!("gh", "api", "#{REPO_PATH}/releases?per_page=100"))
  end

  def tag_commit(tag)
    run!("gh", "api", "#{REPO_PATH}/commits/#{tag}", "--jq", ".sha").strip
  end
end

if $PROGRAM_NAME == __FILE__
  source, head_sha, pipeline = ARGV
  if source.nil? || head_sha.nil? || pipeline.nil?
    warn "usage: nightly_changes.rb <build-source> <head-sha> <pipeline-file>"
    exit 2
  end

  tag = NightlyChanges.latest_nightly_tag(NightlyChanges.fetch_releases)
  last_sha = tag && NightlyChanges.tag_commit(tag)
  puts "Latest nightly: #{tag || 'none'} at #{last_sha || 'n/a'}"
  puts "Head: #{head_sha} (#{source})"

  if NightlyChanges.should_build?(source: source, head_sha: head_sha, last_sha: last_sha)
    puts "Building nightly: uploading #{pipeline}"
    puts NightlyChanges.run!("buildkite-agent", "pipeline", "upload", pipeline)
  else
    puts "No changes since #{tag}; skipping"
  end
end
