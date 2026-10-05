# Cinnabar website

The existing Minecraft styled landing page lives here. Downloads still redirect to
GitHub's latest stable release; Linux displays the release installer command. The
renderer reads the repository's release manifest, title and icon directly, so no
branding or release filename snapshots need synchronization.

```sh
python3 website/render.py
node --test website/tests/app.test.cjs
python3 -m unittest discover -s website/tests -p 'test_*.py' -v
```

`website/dist/` is generated and ignored. The Website workflow verifies pull requests
without using deployment credentials. Changes merged into `dev` to website files,
their branding/release inputs or the workflow deploy automatically. Manual dispatch
can deploy a reviewed branch for verification. App-only changes do not deploy.

Like Zeno Practice, deployment uses GitHub hosted jobs and SSH to this machine.
The `website` environment holds `WEBSITE_SSH_KEY`, `WEBSITE_KNOWN_HOSTS`, and the
`WEBSITE_DEPLOY_HOST` variable. The dedicated `cinnabar` account has a locked Unix
password. Its deployment key permits only the root-owned `deploy/receive.py`
receiver, with forwarding and interactive sessions disabled. It has no sudo or
Docker group membership.

The receiver accepts only the five generated files, rejects paths, links,
duplicates, incomplete archives and oversized payloads, then switches `site/current`
atomically. It serializes publication and retains five releases. The existing
bounded nginx container mounts the release parent; publishing needs no container
restart or privileged command. Forwardme keeps routing `cinnabar.restartfu.com` to
`http://cinnabar-site:80`.

Host setup installs the receiver at `/usr/local/libexec/cinnabar-website-deploy`,
the account's SSH key at `/home/cinnabar/.ssh/authorized_keys`, and nginx config and
Compose file under `/home/danick/deployments/cinnabar-site/deploy/`. Changes to host
receiver or Compose settings require a deliberate administrator installation;
normal website publication updates only static files. Roll back by repointing
`/home/cinnabar/site/current` to a retained release as the account owner.
