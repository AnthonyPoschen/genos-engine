# lint-from-master required workflow

Master's `.github/workflows/lint-from-master.yml` is the lint job a
repository ruleset should require for every pull request, including ones
whose base is a stacked feature branch. GitHub runs that YAML from
`refs/heads/master`, so a stale PR head cannot skip the semgrep venv or
the apt retries (the 10 Nov 2026 PEP 668 / #253 copy-paste loop).

The in-tree `lint` job in `.github/workflows/lint.yml` stays. Its check
name is still required on master. The new job is named `lint-from-master`
on purpose so the two cannot collide. Do not change branch protection
until `lint-from-master` has been green on at least one PR, and do not
remove the old `lint` context until then.

## Ruleset behaviour (as of the GitHub docs, Nov 2026)

- REST: `POST /repos/{owner}/{repo}/rulesets` with a rule of `type:
  workflows`. See [REST API endpoints for rules](https://docs.github.com/en/rest/repos/rules).
- The workflow file is taken from the `ref` in the rule (`refs/heads/master`
  here), not from the PR head or from a mid-stack base.
- The payload targets `~DEFAULT_BRANCH` and `refs/heads/lighting-v3`, not
  `~ALL`. GitHub documents that a `workflows` rule blocks direct pushes
  to every branch it targets, because the workflow runs as part of the
  pull request. `~ALL` would reject every push to a feature branch. The
  cost is that a PR stacked on a feature branch (for example
  `cursor/lighting-integration`) is not covered; retarget it to
  `lighting-v3` or master (see `rebase-after-239.md`).
- Supported events are `pull_request`, `pull_request_target`, and
  `merge_group`. Path, branch, and type filters on those events are
  ignored. The workflow always runs on `opened`, `synchronize`, and
  `reopened`. ([Troubleshooting ruleset workflows](https://docs.github.com/en/enterprise-cloud@latest/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/troubleshooting-rules#troubleshooting-ruleset-workflows))
- A ruleset added while a PR is already open does not run until that PR
  is pushed to, updated, or reopened.
- `evaluate` enforcement is GitHub Enterprise only. This repo uses
  `active` once the file is on master.
- GitHub documents ruleset workflows at the organization and enterprise
  level. This repository is public and user-owned, so the `workflows`
  rule may be rejected (422) on a user account. If it is, there is no
  ruleset route here: either move the repository into an organization
  that can use it, or keep the in-tree `lint` job and rebase stacks as
  `rebase-after-239.md` describes.

## Security

The workflow sets `permissions: contents: read`, checks out
`github.event.pull_request.head.sha` with `persist-credentials: false`,
uses `ubuntu-latest` (not the house runners), and uses no secrets. It
does not use `pull_request_target`.

## Apply

The payload is `docs/ci/ruleset-lint-from-master.json`. The wrapper is
`docs/ci/apply-lint-from-master-ruleset.sh`.

```sh
# Only after lint-from-master.yml is on master:
gh api --method POST repos/AnthonyPoschen/genos-engine/rulesets \
  --input docs/ci/ruleset-lint-from-master.json
```

If a ruleset with that name already exists:

```sh
id=$(gh api repos/AnthonyPoschen/genos-engine/rulesets \
  --jq '.[] | select(.name=="lint-from-master") | .id')
gh api --method PUT "repos/AnthonyPoschen/genos-engine/rulesets/${id}" \
  --input docs/ci/ruleset-lint-from-master.json
```

Creating a ruleset needs the Administration permission on the
repository. The usual `repo` + `workflow` scopes may not be enough; if
the call returns 403 or 404, an admin token or the repo Settings →
Rulesets UI is required.

## After the new job is green

To require `lint-from-master` in addition to the existing checks, and
only later drop `lint`:

```sh
# Add lint-from-master; keep lint and noise-gate.
gh api --method PATCH repos/AnthonyPoschen/genos-engine/branches/master/protection/required_status_checks \
  --input - <<'JSON'
{
  "strict": false,
  "checks": [
    {"context": "lint", "app_id": 15368},
    {"context": "lint-from-master", "app_id": 15368},
    {"context": "noise-gate", "app_id": 15368}
  ]
}
JSON
```

Do not delete the `lint` context in the same step.

## Catch-up of stale heads

This workflow does not rewrite open lighting PRs. After #239 lands,
follow `docs/ci/rebase-after-239.md`.
