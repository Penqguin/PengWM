/**
 * pengwm-edge — the pretty front door for PengWM's install scripts.
 *
 * penqguin.com itself is served by Cloudflare Pages (the `igloo` repo). This
 * Worker claims the dedicated subdomain `pengwm.penqguin.com` (bound as a
 * Custom Domain in wrangler.jsonc) and redirects requests to the canonical
 * sources that live in this repo:
 *
 *   curl -fsSL https://pengwm.penqguin.com/install.sh | bash
 *   curl -fsSL https://pengwm.penqguin.com/uninstall.sh | bash -s -- --yes
 *
 * Redirects are 302 (soft) so the targets can be repointed at any time
 * (e.g. to a Cloudflare R2 mirror) without cache poisoning.
 *
 * Deploy manually from this directory: `npm run deploy` (or `wrangler deploy`).
 */

const REDIRECTS: ReadonlyMap<string, string> = new Map([
  ["/install.sh", "https://raw.githubusercontent.com/Penqguin/PengWM/main/install.sh"],
  ["/uninstall.sh", "https://raw.githubusercontent.com/Penqguin/PengWM/main/uninstall.sh"],
]);

const FALLBACK = "https://github.com/Penqguin/PengWM";

export default {
  fetch(request: Request): Response {
    const { pathname } = new URL(request.url);
    return Response.redirect(REDIRECTS.get(pathname) ?? FALLBACK, 302);
  },
} satisfies ExportedHandler;
