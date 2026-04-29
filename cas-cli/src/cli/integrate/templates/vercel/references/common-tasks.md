# Common Vercel Tasks

Tool-usage shapes for the Vercel MCP. Substitute `{projectId}` / `{teamId}` from the parent SKILL.md `<!-- keep vercel-ids -->` block.

## Pull production runtime logs

```
get_runtime_logs({
  projectId: "{projectId}",
  teamId: "{teamId}",
  environment: "production",
  level: ["error", "fatal"],
  since: "1h"
})
```

## Pull preview/staging logs

```
get_runtime_logs({
  projectId: "{projectId}",
  teamId: "{teamId}",
  environment: "preview",
  since: "1h"
})
```

## Why did the latest deploy fail?

```
1. list_deployments({ projectId: "{projectId}", teamId: "{teamId}" })
2. get_deployment_build_logs({ idOrUrl: "<deploymentId>", teamId: "{teamId}" })
```

## Check current production deployment

```
get_deployment({ idOrUrl: "<latest-deployment-id>", teamId: "{teamId}" })
```
