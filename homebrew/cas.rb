# typed: false
# frozen_string_literal: true

# Homebrew formula for CAS - Coding Agent System
# Install with: brew install codingagentsystem/cas/cas

class Cas < Formula
  desc "Coding Agent System - persistent memory, tasks, rules, and skills for AI agents"
  homepage "https://github.com/codingagentsystem/cas"
  version "0.2.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/codingagentsystem/cas/releases/download/v#{version}/cas-aarch64-apple-darwin.tar.gz"
      sha256 "13ec0b8afd951c6ca75ed4149dda779d7e621336f4cbbdc3551f797d4482feae"
    end
    on_intel do
      odie "CAS does not support Intel macOS. Please use an Apple Silicon Mac."
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/codingagentsystem/cas/releases/download/v#{version}/cas-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "146202ab9b1bdf9c9aa8ec850f4f325b4b6acdc482b54f7fbf707e3177473926"
    end
    on_arm do
      odie "CAS does not support ARM64 Linux."
    end
  end

  def install
    bin.install "cas"
  end

  def caveats
    <<~EOS
      CAS has been installed!

      To get started:
        cas init          # Initialize in your project
        cas serve         # Start the MCP server

      CAS stores data in:
        ~/.config/cas/    (global data)
        .cas/             (project data)

      To update CAS:
        cas update        (self-update)
        brew upgrade cas  (via Homebrew)
    EOS
  end

  test do
    assert_match "cas", shell_output("#{bin}/cas --version")
  end
end
