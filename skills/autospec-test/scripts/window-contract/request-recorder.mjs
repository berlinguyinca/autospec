/**
 * request-recorder.mjs — Playwright route-intercept based request recorder.
 *
 * Attaches a route handler to a Playwright page that captures every request
 * whose URL path matches `pathPattern` (a RegExp or string converted to RegExp).
 * Query parameters are parsed into a `params` object for easy assertion.
 *
 * Usage:
 *   const recorder = attachRecorder(page, '^/api/household/timeline$');
 *   await page.goto(url);
 *   // ... wait for requests ...
 *   inspect(recorder.requests); // [{ url, method, path, params }]
 *
 * Idempotent: calling attachRecorder twice on the same page with the same
 * pattern string does NOT register a duplicate handler (keyed by pattern).
 */

const ATTACHED_KEY = Symbol('autospec:recorder:attached');

/**
 * @param {import('playwright').Page} page
 * @param {string | RegExp} pathPattern  — matched against request.url() pathname only
 * @returns {{ requests: Array<{ url: string, method: string, path: string, params: Record<string, string> }> }}
 */
export function attachRecorder(page, pathPattern) {
  // Normalise pattern to RegExp
  const re = pathPattern instanceof RegExp
    ? pathPattern
    : new RegExp(pathPattern);

  // Idempotency: track attached patterns on the page object itself
  if (!page[ATTACHED_KEY]) {
    page[ATTACHED_KEY] = new Map();
  }

  const key = re.source + re.flags;
  if (page[ATTACHED_KEY].has(key)) {
    return page[ATTACHED_KEY].get(key);
  }

  const recorder = { requests: [] };

  page.route('**/*', (route) => {
    const reqUrl = route.request().url();
    let pathname;
    try {
      pathname = new URL(reqUrl).pathname;
    } catch {
      // Blob / data URLs — skip matching
      return route.continue();
    }

    if (re.test(pathname)) {
      const parsedUrl = new URL(reqUrl);
      const params = {};
      for (const [k, v] of parsedUrl.searchParams.entries()) {
        params[k] = v;
      }
      recorder.requests.push({
        url: reqUrl,
        method: route.request().method(),
        path: pathname,
        params,
      });
    }

    route.continue().catch(() => {});
  });

  page[ATTACHED_KEY].set(key, recorder);
  return recorder;
}
