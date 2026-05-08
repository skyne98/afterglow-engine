---
name: github-browse
description: Browse, search, and read GitHub repositories, issues, PRs, and files using the gh CLI.
---

## Setup

Requires `gh` CLI (authenticated). Check: `gh auth status`

## View a repository

```
gh repo view <owner>/<repo>
```

With structured fields:
```
gh repo view <owner>/<repo> --json name,description,url,stargazerCount,forkCount,primaryLanguage,licenseInfo,homepageUrl,createdAt,pushedAt
```

## Search

Search repositories:
```
gh search repos "agent-browser" --limit 10 --json fullName,description,stargazersCount,language
gh search repos "topic:game-engine language:rust" --limit 10 --json fullName,stargazersCount
```

Search issues/PRs:
```
gh search issues "good first issue" --repo <owner>/<repo> --limit 5 --json title,url,state
gh search issues "label:bug" --repo <owner>/<repo> --json title,labels,state
gh search prs "review:required" --repo <owner>/<repo> --json title,author,createdAt
```

## Read files

README:
```
gh api repos/<owner>/<repo>/readme --jq '.content' | base64 -d
```

Any file:
```
gh api repos/<owner>/<repo>/contents/src/main.rs --jq '.content' | base64 -d
```

Specific branch:
```
gh api repos/<owner>/<repo>/contents/README.md?ref=dev --jq '.content' | base64 -d
```

## List files in a directory

```
gh api repos/<owner>/<repo>/contents/src
```

Recursive file tree:
```
gh api repos/<owner>/<repo>/git/trees/main?recursive=1 --jq '.tree[].path' | head -30
```

## View issues and PRs

```
gh issue view <number> --repo <owner>/<repo>
gh issue view <number> --repo <owner>/<repo> --comments
gh pr view <number> --repo <owner>/<repo>
gh pr view <number> --repo <owner>/<repo> --comments
```

List issues/PRs:
```
gh issue list --repo <owner>/<repo> --limit 10 --state open
gh pr list --repo <owner>/<repo> --limit 10 --state open
```

## View releases

```
gh release list --repo <owner>/<repo> --limit 10
gh release view <tag> --repo <owner>/<repo>
```

## View workflows

```
gh run list --repo <owner>/<repo> --limit 10
gh run view <id> --repo <owner>/<repo>
```

## Combined: explore a repo from scratch

```
# Search for repos
gh search repos "bevy game engine" --limit 5 --json fullName,stargazersCount,description

# View the best match
gh repo view <owner>/<repo> --json description,stargazerCount,primaryLanguage,homepageUrl

# Read the README
gh api repos/<owner>/<repo>/readme --jq '.content' | base64 -d

# List recent issues
gh issue list --repo <owner>/<repo> --limit 5 --state open

# Browse the file tree
gh api repos/<owner>/<repo>/git/trees/main?recursive=1 --jq '.tree[].path' | head -20
```

## Best practices

- Always use `--json` with specific fields for machine-readable output.
- Use `jq` to filter and format API responses.
- Prefer `gh search` over raw API for discovery.
- Use `base64 -d` to decode file content from the API.
- Combine `gh` with `agent-browser` for full GitHub UI access when needed.
