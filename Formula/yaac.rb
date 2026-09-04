class Yaac < Formula
  desc "Terminal Anki client built on Anki's own Rust backend"
  homepage "https://github.com/kiliankoe/yaac"
  url "https://github.com/kiliankoe/yaac.git", tag: "0.1.0"
  license "AGPL-3.0-or-later"
  head "https://github.com/kiliankoe/yaac.git", branch: "main"

  depends_on "protobuf" => :build
  depends_on "rust" => :build

  def install
    # Anki's rslib generates protobuf code at build time.
    ENV["PROTOC"] = formula_opt_bin("protobuf")/"protoc"
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "yaac", shell_output("#{bin}/yaac --version")
  end
end
