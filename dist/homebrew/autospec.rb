class Autospec < Formula
  desc "Multi-harness AI workflow suite: spec → issue tree → autonomous PRs"
  homepage "https://github.com/berlinguyinca/autospec"
  url "https://registry.npmjs.org/@autospec/cli/-/cli-0.1.0.tgz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"

  depends_on "node"

  def install
    system "npm", "install", "-g", "--prefix=#{libexec}", buildpath/"package.json"
    bin.install_symlink Dir["#{libexec}/bin/*"]
  end

  test do
    system bin/"autospec", "--version"
  end
end
