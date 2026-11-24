cask "anibeam" do
  version "2.5.0"
  sha256 "6910e4864502dff24aaf2d4bce3899c240bb9d8c7d0085b649328eb228aef80e"

  url "https://github.com/marcusbandit/Anibeam/releases/download/v#{version}/Anibeam-#{version}-macos-arm64.dmg"
  name "Anibeam"
  desc "Browse, play and track a local anime library"
  homepage "https://github.com/marcusbandit/Anibeam"

  livecheck do
    url :url
    strategy :github_latest
  end

  # The app downloads and installs its own updates
  auto_updates true

  depends_on arch: :arm64
  depends_on macos: :sequoia

  app "Anibeam.app"

  zap trash: [
    "~/Library/Application Support/anibeam",
    "~/Library/Caches/anibeam",
  ]

  caveats <<~EOS
    Anibeam is not notarized, so macOS stops it the first time it opens. Go to
    System Settings > Privacy & Security, scroll to the Anibeam line and click Open Anyway.
  EOS
end
