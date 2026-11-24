# Releases & Automation

Maintainer reference for how Anibeam builds, promotes, and publishes releases. For everyday PRs and local development, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Release tracks

Every build ships from `main`. The version string identifies its update track:

| Build | Version | Comes from | Who gets it |
| --- | --- | --- | --- |
| Stable | `2.5.0` | Release workflow, promoting a nightly | Everyone; the `Latest` GitHub release |
| Nightly | `2.5.0-nightly.20261102.40` | Merge to `main` builds it; `nightly.yml` publishes it | Users on the Nightly track in Settings |
| PR test build | `2.5.0-pr.280.3` | `pr-build.yml` via the `test build` label | Anyone testing a PR from its comment links |
| Phone stable | `android-v2.4.3` | Release workflow (`app=phone`), promoting a phone nightly | Phone app users |
| Phone nightly | `android-v2.4.3-nightly.20261113.41` | Merge to `main` builds it; `nightly.yml` publishes it | Phone nightly testers |

The workspace version in `Cargo.toml` is always the *next* stable version. The phone carries its own `version` in `apps/phone/Cargo.toml`, and its APK `versionCode` is derived from it (`apps/phone/scripts/build.sh`) so nightlies and stable releases upgrade cleanly in order without manual bumps.

Builds outside CI (local checkouts, AUR source builds) derive their display version from `git describe` (`2.4.0.r12.gabc1234`) and compare as the workspace version for update checks.

## Website What's new pages

[anibeam.app/whats-new](https://anibeam.app/whats-new/) is generated directly from `apps/desktop/CHANGELOG.md` and `apps/phone/CHANGELOG.md` via `site/whats-new.py`. `site.yml`, `nightly.yml`, and `release.yml` deploy updates automatically; never edit generated pages by hand.

## Nightlies

Every merge to `main` builds whichever apps changed since their last nightly and retains the artifacts on the workflow run. Running `nightly.yml` from the Actions tab publishes the newest build as a pre-release (or builds `main` first if none exists yet or `force` is ticked), keeping the 10 newest nightlies.

Phone APK and IPA artifacts publish as `android-v*-nightly.*` pre-releases when phone or core files changed. Publishing uses the `NIGHTLY_TOKEN` secret (a fine-grained token with Contents and Workflows write permissions) so tags succeed even after workflow edits.

## PR test builds

Add the `test build` label to a PR to publish installers on the `pr-builds` pre-release with download links commented on the PR. Every push rebuilds them; closing the PR or removing the label cleans them up.

## Cutting a stable release

### Desktop

1. Curate the user-facing notes under the newest heading in `apps/desktop/CHANGELOG.md` (use `.github/changelog-draft.sh desktop` to review commits since the last stable tag).
2. Run the **Release** workflow (`release.yml`) on `main` from the Actions tab. It promotes the newest nightly commit (so what ships has already been tested), waits for approval in the `release` environment, builds all desktop targets, publishes the release, and commits the next minor version + changelog heading to `main`.
   - If a workflow file changed on `main` since the last nightly, trigger a forced Nightly first so the commit can be tagged.
3. Update the AUR packages and Homebrew cask:
   ```bash
   apps/desktop/packaging/aur/publish.sh <version>
   apps/desktop/packaging/brew/bump.sh <version>
   ```

### Phone

1. Curate notes under the newest heading in `apps/phone/CHANGELOG.md` (`.github/changelog-draft.sh phone`).
2. Run **Release** (`release.yml`) with `app=phone`. It promotes the newest phone nightly to `android-v<version>` (never marking it as `Latest`, which stays desktop's for the site and `install.sh`) and bumps `apps/phone/Cargo.toml` + `apps/phone/CHANGELOG.md`.

Every released Android APK is signed with `ANIBEAM_ANDROID_KEYSTORE` (base64 keystore secret) so updates install cleanly over prior builds. Never publish phone builds by hand.

## Hotfixes

When a shipped desktop release needs a patch:

1. Create a release branch from its tag:
   ```bash
   git push origin v2.5.0^{commit}:refs/heads/release/2.5
   ```
2. Land the fix on `main` first via a normal PR, then cherry-pick it (`git cherry-pick -x`) in a PR targeting `release/2.5`, bumping the version to `2.5.1` and adding a short entry to `apps/desktop/CHANGELOG.md`.
3. Tag and push: `git tag v2.5.1 origin/release/2.5 && git push origin v2.5.1`. Never merge the release branch back into `main`.

## Automerge workflow

Maintainer PRs carrying both `full ci` and `automerge` labels are squash-merged by `.github/workflows/ios-automerge.yml` (`.github/ios-automerge.sh`) once:

- the PR is ready for review and targets `main`;
- the latest run of every workflow on the head commit passed or skipped;
- `Full CI` actually built and passed on the head commit (and `PR build` passed if `test build` is set).

If a check fails, the PR stays open until a follow-up commit turns checks green. Removing `automerge` pauses automatic merging. Contributors listed in the `IOS_AUTOMERGE_AUTHORS` repository variable may also have macOS/iOS-only PRs merged automatically once checks pass.
