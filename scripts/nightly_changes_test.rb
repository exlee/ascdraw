#!/usr/bin/env ruby
# frozen_string_literal: true

require "minitest/autorun"
require_relative "nightly_changes"

class NightlyChangesTest < Minitest::Test
  def release(tag, created_at, draft: false)
    { "tag_name" => tag, "created_at" => created_at, "draft" => draft }
  end

  def test_picks_newest_nightly_release
    releases = [
      release("nightly-100", "2026-10-01T03:40:00Z"),
      release("nightly-300", "2026-10-03T03:40:00Z"),
      release("nightly-200", "2026-10-02T03:40:00Z")
    ]

    assert_equal "nightly-300", NightlyChanges.latest_nightly_tag(releases)
  end

  def test_ignores_drafts_and_other_releases
    releases = [
      release("nightly-100", "2026-10-01T03:40:00Z"),
      release("nightly-200", "2026-10-02T03:40:00Z", draft: true),
      release("v1.0.0", "2026-10-03T03:40:00Z")
    ]

    assert_equal "nightly-100", NightlyChanges.latest_nightly_tag(releases)
  end

  def test_no_tag_without_nightly_releases
    assert_nil NightlyChanges.latest_nightly_tag([release("v1.0.0", "2026-10-03T03:40:00Z")])
  end

  def test_schedule_skips_unchanged_commit
    refute NightlyChanges.should_build?(event: "schedule", head_sha: "abc", last_sha: "abc")
  end

  def test_schedule_builds_changed_commit
    assert NightlyChanges.should_build?(event: "schedule", head_sha: "def", last_sha: "abc")
  end

  def test_schedule_builds_without_previous_nightly
    assert NightlyChanges.should_build?(event: "schedule", head_sha: "abc", last_sha: nil)
  end

  def test_manual_run_always_builds
    assert NightlyChanges.should_build?(event: "workflow_dispatch", head_sha: "abc", last_sha: "abc")
  end
end
