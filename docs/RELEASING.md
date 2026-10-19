# Making a release

A release is one pushed tag. The [release workflow](../.github/workflows/release.yml) does the rest: it builds the installer on GitHub's own runner, publishes the GitHub release with the installer, its SHA-256 and the changelog entry, and sends the new version to winget.

## Steps

1. Set the version, in all three places at once:
   ```
   node scripts/bump-version.mjs 1.0.1
   cargo check --workspace
   ```
2. Add the entry to `CHANGELOG.md` under a heading `## [1.0.1] - YYYY-MM-DD`. The release notes are taken from it, so the release fails without it.
3. Commit, and push `main`.
4. Tag that commit and push the tag:
   ```
   git tag v1.0.1
   git push origin v1.0.1
   ```
5. Watch the run under Actions › Release. It takes about half an hour. When it is green, the release is on the Releases page and a pull request is open on `microsoft/winget-pkgs`; winget shows the new version once Microsoft's checks have merged it, usually within a day.

The tag must equal the version in the tree (`v1.0.1` for `1.0.1`); the workflow stops at once when it does not. Running the workflow again for the same tag replaces the installer on the existing release. Each release carries the installer twice: under its versioned name (winget uses that one) and as `meteor-setup.exe`, which the website's download button falls back to.

The website is not part of the pipeline: its download card reads the latest release from GitHub by itself, so a release needs no change there.

## One-time setup for winget

The first version of the package was submitted by hand. For the workflow to send later versions, the repository needs one secret:

1. On GitHub, create a classic personal access token with the `public_repo` scope only (Settings › Developer settings › Personal access tokens › Tokens (classic)).
2. Save it as the secret `WINGET_TOKEN` of this repository (Settings › Secrets and variables › Actions), or from a terminal: `gh secret set WINGET_TOKEN --repo shakibbinkabir/lintel` and paste the token when asked.

Without the secret the release still builds and publishes; only the winget step is skipped, with a notice saying so.

## Building without releasing

Actions › Build › Run workflow builds the installer from any branch and keeps it as an artifact of the run, without publishing anything.

## Code signing

Releases are unsigned until the SignPath Foundation grants the project a certificate (see the code signing policy in the README). When it does, the signing request is added to the release workflow at the marked place, and every signed installer needs its release approved in SignPath.
