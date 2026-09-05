#!/usr/bin/env ruby
# frozen_string_literal: true

require "minitest/autorun"
require_relative "check_glibc_version"

class GlibcVersionCheckTest < Minitest::Test
  OBJDUMP_SAMPLE = <<~OUT
    DYNAMIC SYMBOL TABLE:
    0000000000000000      DF *UND*	0000000000000000  GLIBC_2.2.5 malloc
    0000000000000000      DF *UND*	0000000000000000  GLIBC_2.14  memcpy
    0000000000000000      DF *UND*	0000000000000000  GLIBC_2.39  __isoc23_strtol
    0000000000000000      DF *UND*	0000000000000000  GLIBC_2.2.5 free
    0000000000000000  w   D  *UND*	0000000000000000              __gmon_start__
  OUT

  def test_collects_each_required_version_once
    assert_equal %w[2.2.5 2.14 2.39],
                 GlibcVersionCheck.required_versions(OBJDUMP_SAMPLE)
  end

  def test_no_versions_for_output_without_glibc_tags
    assert_empty GlibcVersionCheck.required_versions("DYNAMIC SYMBOL TABLE:\n")
  end

  def test_reports_versions_above_the_maximum
    versions = GlibcVersionCheck.required_versions(OBJDUMP_SAMPLE)

    assert_equal %w[2.39], GlibcVersionCheck.newer_than_max(versions, "2.35")
  end

  def test_maximum_itself_is_allowed
    assert_empty GlibcVersionCheck.newer_than_max(%w[2.35], "2.35")
  end

  def test_compares_components_numerically_not_lexically
    assert_empty GlibcVersionCheck.newer_than_max(%w[2.9], "2.35")
    assert_equal %w[2.100], GlibcVersionCheck.newer_than_max(%w[2.100], "2.35")
  end

  def test_shorter_prefix_version_is_below_its_extension
    assert_empty GlibcVersionCheck.newer_than_max(%w[2.2], "2.2.5")
    assert_equal %w[2.2.5], GlibcVersionCheck.newer_than_max(%w[2.2.5], "2.2")
  end

  def test_sorts_by_numeric_components
    assert_equal %w[2.2.5 2.14 2.39],
                 GlibcVersionCheck.sort_versions(%w[2.39 2.14 2.2.5])
  end
end
