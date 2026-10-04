# pengwm-edge

A one-file Cloudflare Worker that turns `pengwm.penqguin.com` into the copy-paste
install front door for PengWM:

```sh
curl -fsSL https://pengwm.penqguin.com/install.sh | bash
curl -fsSL https://pengwm.penqguin.com/uninstall.sh | bash -s -- --yes
```

## What it does

| Path | Redirects to |
| --- | --- |
| `/install.sh` | `https://raw.githubusercontent.com/Penqguin/PengWM/main/install.sh` |
| `/uninstall.sh` | `https://raw.githubusercontent.com/Penqguin/PengWM/main/uninstall.sh` |
| anything else | `https://github.com/Penqguin/PengWM` |

Redirects are `302` so targets can be repointed later (e.g. R2 mirror) without
poisoning caches. The scripts are *not* copies — their canonical source stays in
this repo; the resolved `main` happens to be what's deployed, and the scripts
themselves resolve the latest GitHub release at install time.

## Deploying

Deployments are manual (the redirect changes ~never). From this directory:

```sh
npm install        # once
npm run deploy     # wrangler deploy; creates the custom-domain DNS record on first run
npm run dev        # optional local dev at wrangler dev default URL
```

Requires a one-time `wrangler login` (OAuth in your browser). Do **not** add a
CI auto-deploy for this — see docs/distribution.md.
