#!/usr/bin/env bash
# Merges the open PRs that are ready to go in on their own. ios-automerge.yml runs it.
#
# Two kinds of PR merge here, both ready for review and aimed at main:
#
# - The owner's PR labelled `automerge` and `full ci` (#1414) is squash merged once every
#   check and workflow run on its head commit has passed and Full CI has built that head
#   and passed. A run of a label-gated workflow (Full CI, PR build) whose jobs all skipped,
#   such as one another label started, proves nothing and does not count. The squash commit is titled the way GitHub titles it
#   by default, the PR title plus " (#N)", with GitHub's default body.
# - A PR by one of AUTHORS, the iPhone and Mac contributors (#909), is squash merged once
#   every check and workflow run on its head commit has passed, when every file it changes
#   is iPhone or Mac only (ONLY below). A PR of theirs that changes anything else is its
#   author's to merge, and this leaves it alone (#916).
#
#   GH_REPO      owner/name
#   OWNER        the owner's login; by default the owner part of GH_REPO
#   AUTHORS      the contributors' GitHub logins, separated by spaces or commas; may be empty
#   PRS          the PR numbers to look at, from the event; empty looks at every open one
#   MERGE_TOKEN  a token that merges the owner's PRs, so main's push workflows start; when it
#                is empty or may not merge, GH_TOKEN merges and those workflows do not start
#   DRY_RUN      0 to merge; by default it only says what it would do
#
# It reads PRs through the API only and never runs their code. Anything it cannot read
# leaves the PR waiting: an API error never counts as a pass. A red check leaves the PR
# open without a comment; the owner's CI watcher reports it.
set -euo pipefail
: "${GH_REPO:?}"
OWNER="${OWNER:-${GH_REPO%%/*}}"
AUTHORS="${AUTHORS:-}"
PRS="${PRS:-}"
DRY_RUN="${DRY_RUN:-1}"
MERGE_TOKEN="${MERGE_TOKEN:-}"
SELF_PATH=".github/workflows/ios-automerge.yml"
SELF_JOB="automerge"
FULL_CI_PATH=".github/workflows/full-ci.yml"
# Workflows that build only when their label lands or a push comes while it is on. Any other
# label landing starts a run of them whose gate passes and whose other jobs all skip.
GATED=("$FULL_CI_PATH" ".github/workflows/pr-build.yml")
LABEL="automerge"
FULL_CI_LABEL="full ci"

# Paths that only the iPhone or the Mac build reads. Everything else can change another
# platform: Cargo.toml and Cargo.lock, and every workflow, this one included. android.yml's
# pull_request paths exclude the same iPhone paths as globs (#950): change both together.
ONLY='^(apps/phone/ios/'
ONLY+='|apps/phone/src/(.+/)?(ios|metal)(\.rs$|/)'
ONLY+='|apps/phone/scripts/[^/]*ios[^/]*$'
ONLY+='|apps/(desktop|linux)/packaging/macos/'
ONLY+=')'

# The log goes to stderr: consider's stdout carries only the branches it merged.
say() { echo "#$1: $2" >&2; }

PASSED='IN("success", "skipped", "neutral")'

# Picks the run of the label-gated workflow PATH that speaks for the commit and prints
# "<run id> built" or "<run id> skipped", or nothing when the workflow never ran on it. The
# newest run that did more than its gate speaks for it, so a run another label started
# later, whose jobs all skipped, can neither hide a red build nor stand in for a missing
# one. With no such run, the newest run speaks. A run still going may yet build, so it fails
# until it is done.
gated_run() {
  local path="$1" runs="$2" ids id jobs
  ids="$(jq -r --arg path "$path" \
    'map(select(.path == $path)) | sort_by([.created_at, .id]) | reverse | .[].id' <<<"$runs")"
  [ -n "$ids" ] || return 0
  if jq -e --arg path "$path" \
    'any(.[]; .path == $path and .status != "completed")' <<<"$runs" >/dev/null; then
    echo "    ${path##*/} still running" >&2
    return 1
  fi
  for id in $ids; do
    jobs="$(gh api --paginate "repos/$GH_REPO/actions/runs/$id/jobs?filter=latest&per_page=100" \
      --jq '.jobs[]')" || return 1
    # The gate always runs; a run built when any other job did not skip. full-ci.yml and
    # pr-build.yml keep their gate job named gate for this. Full CI's no-app job counts too:
    # it runs instead of the builds when the pull request changes no app (#1548).
    if jq -se 'any(.[]; .name != "gate" and .conclusion != "skipped")' <<<"$jobs" >/dev/null; then
      echo "$id built"
      return 0
    fi
  done
  echo "${ids%%$'\n'*} skipped"
}

# Every check run and workflow run that speaks for the commit finished as passed or skipped,
# and there is at least one run: a push whose CI has not started is not passed. The newest
# run of each workflow speaks for it, a label-gated one's as gated_run picks; a run it
# replaced, such as one cancelled by a newer one, and that run's checks, no longer count.
# With the second argument 1, Full CI must have built the commit as well.
checks_pass() {
  local sha="$1" need_full="$2" checks all runs gated path picked ids="" keep dropped bad
  checks="$(gh api --paginate "repos/$GH_REPO/commits/$sha/check-runs?per_page=100" \
    --jq '.check_runs[]')" || return 1
  all="$(gh api --paginate "repos/$GH_REPO/actions/runs?head_sha=$sha&per_page=100" \
    --jq '.workflow_runs[]')" || return 1
  all="$(jq -sc . <<<"$all")"
  runs="$(jq -c --arg self "$SELF_PATH" 'map(select(.path != $self))' <<<"$all")"
  if [ "$(jq length <<<"$runs")" = 0 ]; then
    echo "    no workflow runs yet" >&2
    return 1
  fi
  for path in "${GATED[@]}"; do
    picked="$(gated_run "$path" "$runs")" || return 1
    if [ "$path" = "$FULL_CI_PATH" ] && [ "$need_full" = 1 ] && [ "${picked#* }" != built ]; then
      echo "    Full CI has not built this commit" >&2
      return 1
    fi
    ids+="${picked%% *} "
  done
  gated="$(printf '%s\n' "${GATED[@]}" | jq -R . | jq -sc .)"
  keep="$(jq -c --argjson gated "$gated" --arg ids "$ids" '
      ($ids | split(" ") | map(select(. != ""))) as $picked
      | (map(select(.path | IN($gated[]) | not)) | group_by(.workflow_id)
          | map(max_by([.created_at, .id])))
      + map(select((.id | tostring) | IN($picked[])))' <<<"$runs")"
  # The check suites of the runs that no longer count, this workflow's own among them. Its
  # own job is dropped by name too, whatever commit its run is filed under.
  dropped="$(jq -c --argjson kept "$(jq -c 'map(.check_suite_id)' <<<"$keep")" \
    'map(.check_suite_id) - $kept' <<<"$all")"
  bad="$(jq -r --argjson dropped "$dropped" --arg self "$SELF_JOB" '
      select(.name != $self)
      | select(.check_suite.id as $s | $dropped | index($s) | not)
      | select(.status != "completed" or (.conclusion | '"$PASSED"' | not))
      | "\(.name) \(.status) \(.conclusion)"' <<<"$checks")"
  bad+=$'\n'"$(jq -r ".[] | select(.status != \"completed\" or (.conclusion | $PASSED | not))
      | \"\(.name) \(.status) \(.conclusion)\"" <<<"$keep")"
  bad="$(sed '/^$/d' <<<"$bad")"
  [ -z "$bad" ] || { echo "$bad" | sed 's/^/    /' >&2; return 1; }
}

# Squash merges the owner's PR through the API, pinned to the head checked above, so a push
# after the checks stops the merge. MERGE_TOKEN merges when it may, so main's push workflows
# start; a merge by GH_TOKEN, the workflow's own token, starts none.
merge_owner() {
  local pr="$1" sha="$2" title="$3" out
  local args=(-X PUT "repos/$GH_REPO/pulls/$pr/merge" -f merge_method=squash -f sha="$sha"
    -f commit_title="$title (#$pr)")
  if [ -n "$MERGE_TOKEN" ]; then
    if out="$(GH_TOKEN="$MERGE_TOKEN" gh api "${args[@]}" 2>&1 >/dev/null)"; then
      return 0
    fi
    case "$out" in
      *"HTTP 401"* | *"HTTP 403"* | *"HTTP 404"*)
        say "$pr" "MERGE_TOKEN may not merge (${out##*(}, merging with the workflow's token; main's push workflows will not start"
        ;;
      *)
        echo "$out" >&2
        return 1
        ;;
    esac
  else
    say "$pr" "no MERGE_TOKEN, merging with the workflow's token; main's push workflows will not start"
  fi
  gh api "${args[@]}" >/dev/null
}

# Looks at one PR and merges it when it is ready. Prints the PRs stacked on one it merged.
consider() {
  local pr="$1" info sha kind files outside stacked merged
  info="$(gh pr view "$pr" \
    --json state,isDraft,baseRefName,author,headRefOid,headRefName,mergeable,labels,title)" ||
    { say "$pr" "could not read the PR"; return 0; }
  jq -e '.state == "OPEN" and (.isDraft | not) and .baseRefName == "main"' <<<"$info" >/dev/null ||
    return 0
  sha="$(jq -r .headRefOid <<<"$info")"
  if jq -e --arg owner "$OWNER" --arg label "$LABEL" \
    '.author.login == $owner and any(.labels[]; .name == $label)' <<<"$info" >/dev/null; then
    kind=owner
    if ! jq -e --arg label "$FULL_CI_LABEL" 'any(.labels[]; .name == $label)' <<<"$info" >/dev/null; then
      say "$pr" "labelled $LABEL without $FULL_CI_LABEL, waiting for both"
      return 0
    fi
  elif jq -e --argjson authors "$AUTHORS_JSON" '.author.login | IN($authors[])' \
    <<<"$info" >/dev/null; then
    kind=contributor
    # A renamed file counts at its old path too, so a shared file moved under ios/ still counts.
    files="$(gh api --paginate "repos/$GH_REPO/pulls/$pr/files" \
      --jq '.[] | .filename, (.previous_filename // empty)')" ||
      { say "$pr" "could not list the files"; return 0; }
    if [ -z "$files" ]; then
      say "$pr" "no files listed"
      return 0
    fi
    outside="$(grep -Ev "$ONLY" <<<"$files" | sort -u || true)"
    if [ -n "$outside" ]; then
      say "$pr" "changes $(wc -l <<<"$outside") file(s) outside iPhone and Mac, left to its author"
      return 0
    fi
  else
    return 0
  fi
  if ! checks_pass "$sha" "$([ "$kind" = owner ] && echo 1 || echo 0)"; then
    say "$pr" "checks not passed yet on ${sha:0:8}"
    return 0
  fi
  if [ "$(jq -r .mergeable <<<"$info")" != MERGEABLE ]; then
    say "$pr" "not mergeable into main"
    return 0
  fi
  if [ "$DRY_RUN" != 0 ]; then
    say "$pr" "would merge ${sha:0:8} ($kind)"
    return 0
  fi
  # Listed before the merge, since deleting the branch moves them onto main.
  stacked="$(gh pr list --state open --base "$(jq -r .headRefName <<<"$info")" \
    --json number --jq '.[].number' || true)"
  if [ "$kind" = owner ]; then
    merge_owner "$pr" "$sha" "$(jq -r .title <<<"$info")" && merged=1 || merged=0
  else
    # --match-head-commit: a push after the checks above stops the merge.
    gh pr merge "$pr" --squash --match-head-commit "$sha" >&2 && merged=1 || merged=0
  fi
  if [ "$merged" = 1 ]; then
    say "$pr" "merged ${sha:0:8}"
    echo "$stacked"
  else
    say "$pr" "merge refused"
  fi
}

AUTHORS_JSON="$(tr ', ' '\n\n' <<<"$AUTHORS" | sed '/^$/d' | jq -R . | jq -sc .)"
if [ -z "$PRS" ]; then
  PRS="$(gh pr list --state open --base main --limit 200 --json number --jq '.[].number')"
fi

# A merge deletes its branch, which moves a PR stacked on it onto main, so those PRs are
# looked at next, a few times over for a stack.
for _ in 1 2 3; do
  stacked=""
  for pr in $PRS; do
    stacked+="$(consider "$pr") "
  done
  [ -n "${stacked//[[:space:]]/}" ] || break
  sleep 10
  PRS="$stacked"
done
