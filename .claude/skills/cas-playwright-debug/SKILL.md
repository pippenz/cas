---
name: cas-playwright-debug
description: Debug and fix Playwright E2E test failures. Use when investigating failing Playwright tests, fixing stale locators, route mock mismatches, auth seeding issues, or parallel worker race conditions.
user-invocable: false
---

# playwright-debug

# Playwright Test Debugging & Fixing

## Quick Diagnosis Checklist

When a Playwright test fails, check these in order:

### 1. Read the Error Context
```bash
cat test-results/<test-folder>/error-context.md
```
- **Sign-in page?** → Auth/localStorage seeding failed or API mock missing
- **Empty page?** → Route mocks not intercepting, data not loading
- **Wrong content?** → Locators stale, UI changed since test was written

### 2. Route Mock URL Mismatch (Most Common Cause)
The #1 cause of failures: test mocks intercept `localhost:3001` but the app's actual backend URL is different (e.g., Vercel deployment URL from env vars).

**Bad** — hardcoded origin:
```ts
await page.route(/http:\/\/localhost:3001\/api\/users/, route => route.fulfill({...}));
```

**Good** — origin-agnostic pattern:
```ts
await page.route(/.*\/api\/users/, route => route.fulfill({...}));
// Or use glob:
await page.route('**/api/users', route => route.fulfill({...}));
```

**How to check**: Look at `NUXT_PUBLIC_SERVER_URL` or equivalent env var. Compare against what the test mocks intercept. If they differ, the mocks silently miss every request.

### 3. Auth Seeding with addInitScript
For tests that seed auth via localStorage (no real Firebase):

```ts
async function seedLocalStorage(page, accountOverrides = {}) {
  const account = { ...BASE_ACCOUNT, ...accountOverrides };
  await page.addInitScript((storageValue) => {
    localStorage.setItem('app-auth', storageValue);
  }, JSON.stringify({ account }));
}
```

**Key rules:**
- `addInitScript` runs BEFORE page scripts — must be called BEFORE `page.goto()`
- Seeds persist for the page lifetime (survives navigations)
- Must mock ALL API endpoints the page calls on load, or unmocked calls may fail and redirect to sign-in

### 4. Parallel Worker Race Conditions
Tests pass with `--workers=1` but fail in parallel:

**Causes:**
- Dev server too slow under load → pages timeout
- Shared state bleeding between contexts
- Port conflicts or server connection limits

**Fixes:**
- Increase timeouts: `test.setTimeout(30_000)`
- Add `await page.waitForLoadState('networkidle')` after goto
- Use `test.describe.serial` for tests that must run in order

### 5. Stale Locators
UI changed but test selectors weren't updated:

```ts
// Bad — brittle CSS selector
page.locator('.btn-primary.submit-form')

// Good — semantic role-based
page.getByRole('button', { name: 'Submit' })

// Good — data-testid
page.getByTestId('submit-button')

// Debug: check how many elements match
const count = await page.locator('your-selector').count();
```

**Strict mode**: `getByText('Foo')` fails if multiple elements match. Fix with `{ exact: true }` or `.first()`.

### 6. Mock Coverage for addInitScript Tests
Mock EVERY endpoint the page calls on load:

```ts
async function installMocks(page) {
  // Auth token refresh — origin-agnostic
  await page.route(/.*securetoken\.googleapis\.com.*/, route => route.fulfill({
    status: 200, contentType: 'application/json',
    body: JSON.stringify({ id_token: 'fake', refresh_token: 'fake', expires_in: '3600' }),
  }));
  // Firebase user lookup
  await page.route(/.*identitytoolkit\.googleapis\.com.*/, route => route.fulfill({
    status: 200, contentType: 'application/json',
    body: JSON.stringify({ users: [{ localId: 'uid', email: 'test@test.com', emailVerified: true }] }),
  }));
  // App API endpoints — ALWAYS use origin-agnostic patterns
  await page.route(/.*\/accounts\/me/, route => route.fulfill({...}));
  await page.route(/.*\/notifications/, route => route.fulfill({...}));
  await page.route(/.*\/feature-flags\/public/, route => route.fulfill({...}));
}
```

**Missing mocks** → requests hit real (non-running) backend → connection refused → error handler may redirect to sign-in.

## Running Tests

```bash
# Run specific test file
npx playwright test tests/path/to/test.spec.ts

# Run single test by name
npx playwright test -g "test name substring"

# Sequential (debug race conditions)
npx playwright test --workers=1

# Show HTML report after run
npx playwright show-report

# List tests without running
npx playwright test --list
```

## Ozer Project Config
Tests are organized into projects in `playwright.config.ts`:
- **telehealth-setup**: Auth setup (runs first, saves storageState)
- **telehealth**: Main tests (depend on setup for auth)
- **telehealth-ui**: Tests using addInitScript (NO setup dependency, self-contained)
- **unauthenticated**: Tests without auth

`telehealth-ui` tests seed auth via localStorage, not storageState files. They need their own complete route mock coverage.

## Response Shape Changes
When backend API response shapes change, update:
1. The test's route mock to return the new shape
2. Any assertions checking response data
3. Frontend consumers that may need backwards-compatible handling

## Instructions

playwright-debug

## Tags

testing, playwright, e2e, debugging
