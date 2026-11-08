# Your first goatnet

You will stand up a working goatnet on one Ubuntu host or VM on your LAN and use it: enrol your laptop, sign in, read its health, and publish a message and receive it. You write one file of four facts and run one command. What comes up is a real goatnet with its own mesh, identity service, certificate authority and data fabric, not a demo. You rehearse the run on a worked example first, on your laptop.

- **Audience:** Someone who has never run a goatnet and knows none of the software under it
- **Time:** The rehearsal is short. The real run takes an hour or more; downloads, reading and fixing mistakes all add to it, so do not plan around a number
- **You need:** One Ubuntu 24.04 host or VM with 4 cores and 16 GB that you can SSH into with a key, a laptop on the same LAN, the `goatbuilder` and `goat` binaries, and a checkout of the goat repository
- **You finish with:** A goatnet whose acceptance gate is green and on which you have published a message and received it
- **Status:** Updated 2026-11-02 during the first measured run; that run's fixes are in this text
- **Next:** [Tutorial 1, a new goatnet, from nothing](01-new-goatnet.md)

goatlite is `preset: lite` in the goatbuilder configuration. It runs the same fourteen-phase standup every goatnet runs, in its LAN profile, with every decision already made ([goatlite.md](../design/goatlite.md), [ADR 1213](../adr/1213-goatlite-preset-one-box-one-file.md)). The procedure behind every phase is the standup runbook, [goatnet-standup.md](../operations/goatnet-standup.md); this tutorial is one instance of it, not a second procedure. The script that stands beside the builder, `ops/scripts/goat-standup.sh`, runs the same phases and stays documented.

Until ADR 1197's public images land, pulling the goatnet's container images needs one account: a GitHub token with `read:packages`, exported as `GHCR_READ_TOKEN`. Nothing else in this tutorial needs an account with any outside service.

## The words used here

- **goatnet**: one environment: its own mesh, identity, certificate authority and data fabric.
- **the box**: the one host or VM the goatnet runs on.
- **zone**: the DNS name under which the goatnet's services live. goatlite's is `<name>.lan`, served by the box itself.
- **phase**: one numbered, re-runnable step of the standup. A failed phase is fixed and resumed by number.
- **assay**: the acceptance gate. Every provisioned part is proven live and working, layer by layer.
- **the doctor**: `goatbuilder doctor`, the command that checks your laptop and your files, and prints the fix for anything missing.
- **the worked example**: a finished goatnet's files, kept in the repository. The standup can run against them, so you see the whole run without touching a box.

## Where each command runs

Three places. Mixing them up is the most common mistake, so every command block below starts with the place it runs.

- **On your laptop:** Your own terminal, at the root of the goat checkout. `goatbuilder` and `goat` run here and reach the box over SSH.
- **On the box over SSH:** A terminal on the box, as the `ssh_user` from your file. Needed only twice: to check the disk name and to read your one-time password.
- **In the browser:** The goatnet's dashboard, once, at the step that creates your account.

You never run `goatbuilder` on the box. It runs on your laptop and does everything to the box over SSH.

## 1. Get the tools, the checkout and the two programs

### First, install the tools

The run uses these programs on your laptop. Install them now, so nothing stops you partway through. Section 4's doctor command checks the laptop and prints the install command for anything it finds missing.

The laptop can run Linux or macOS. ⚠️ Windows itself is not supported yet. WSL runs the Linux path and may work, but no run has been tested on it. The Linux commands below are for Ubuntu 24.04, and CI runs them as written on a fresh Ubuntu 24.04 on every change to this page (`.github/workflows/goatlite-workstation.yml`).

| Program                                                   | Used for                                                                                           |
| --------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `goatbuilder` | runs the standup from your laptop; built from the checkout in the next step |
| `goat` | the client you use the goatnet with; built in the same step |
| `netbird` | joins your laptop to the goatnet's mesh in section 6; install it from `https://docs.netbird.io/how-to/installation` |
| `git`                                                   | the checkout                                                                                       |
| `ssh`                                                   | reaching the box                                                                                   |
| `cargo` (Rust)                                          | building `goatbuilder` and `goat`; install it from `https://rustup.rs/`                            |
| `yq`, version 4, by mikefarah                           | reading the certificate manifest when phase 1 mints the goatnet's certificate authority. The `yq` that apt installs is a different program with the same name |
| `age`                                                   | encrypting the certificate authority's key                                                         |
| `go` 1.26 or newer, `pkg-config` and the PC/SC headers  | building the certificate tools phase 1 runs (`ops/enrollment`). The PC/SC headers are `libpcsclite-dev` on Linux; macOS has them built in |
| `python3` with `venv`                                   | one Python environment for this tutorial, set up in the checkout below                            |
| `ansible-playbook`                                      | every deploy step on the box; `ansible-core` 2.20.11 from pip, inside that environment            |
| `pyyaml`, `jsonschema`, `jmespath`                      | the render, the checks, and the access-rule steps; from pip, inside that environment              |
| `node`, `npm`, Playwright with Chromium               | the two acceptance checks that drive a browser                                                     |

#### On Ubuntu 24.04

Refresh the package lists, then install the packages. A fresh image has no package lists yet, so `apt install` on its own cannot find them:

```bash
sudo apt update
sudo apt install -y git openssh-client curl ca-certificates build-essential python3 python3-venv nodejs npm age pkg-config libpcsclite-dev
```

Install mikefarah's `yq`, version 4. Ubuntu's `yq` package is a different program; do not install it:

```bash
sudo curl -fsSL -o /usr/local/bin/yq "https://github.com/mikefarah/yq/releases/download/v4.44.3/yq_linux_$(dpkg --print-architecture)"
sudo chmod 0755 /usr/local/bin/yq
yq --version
```

Install Go from `go.dev`. Ubuntu's `golang-go` is older than the version `ops/enrollment/go.mod` asks for:

```bash
curl -fsSL "https://go.dev/dl/go1.26.10.linux-$(dpkg --print-architecture).tar.gz" | sudo tar -C /usr/local -xz
export PATH="/usr/local/go/bin:$PATH"
go version
```

Add `export PATH="/usr/local/go/bin:$PATH"` to your shell profile too, so a new terminal finds `go`.

Install Rust with `rustup` from `https://rustup.rs/`; it is not in this block because it asks questions as it installs.

#### On macOS

```text
brew install git python node yq age go rustup
```

The Python environment below uses Homebrew's `python3`; macOS's own is too old for `ansible-core` 2.20. Check with `python3 --version`, which must print 3.12 or newer.

One registry token is also needed before images can be pulled; section 4 names it and where to put it.

### Get the checkout and build the two programs

Get the checkout. Every later command runs from its root:

```text
git clone https://github.com/usareur-af/goatcore.git
cd goatcore
```

There is no release page yet, so build both programs from the checkout with Rust. The first build takes several minutes.

```text
(cd crates && cargo build --release --locked -p goatbuilder -p goat-cli)
mkdir -p ~/.local/bin
install -m 0755 crates/target/release/goatbuilder crates/target/release/goat ~/.local/bin/
goatbuilder --version
goat --version
```

If `--version` says "command not found", add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile and open a new terminal.

### Set up Python, Ansible and the browser

Every command here runs on your laptop, from the root of the checkout.

Create one Python environment for this tutorial and install its modules and Ansible into it. Ubuntu 24.04 refuses `pip install` into the system Python, and the environment keeps the tutorial's versions apart from anything else on the laptop:

```bash
python3 -m venv ~/.venvs/goatlite
. ~/.venvs/goatlite/bin/activate
python3 -m pip install pyyaml jsonschema jmespath 'ansible-core==2.20.11'
ansible-galaxy collection install -r ops/ansible/requirements.yml
```

`ansible-core` 2.20 is the newest line the repository's Ansible CI accepts (`.github/workflows/ansible-syntax-check.yml`). It carries no collections of its own; `ops/ansible/requirements.yml` names the ones the plays use.

⚠️ Every terminal you use for this tutorial activates the environment first, with `. ~/.venvs/goatlite/bin/activate`. `goatbuilder` runs `python3` and `ansible-playbook` from your `PATH`, so in a terminal without the environment the first phase that needs them fails.

Install the browser for the two browser checks. `--with-deps` also installs the system libraries Chromium needs on Linux, and asks for your password to do it:

```bash
(cd ops/goat-id-console/web/e2e && npm ci && npx playwright install --with-deps chromium)
```

Check that the browser starts. It prints `chromium launched`; an error naming a missing library, such as `libatk-1.0.so.0`, means the system libraries are not installed, so run the line above again:

```bash
(cd ops/goat-id-console/web/e2e && node -e "import('@playwright/test').then(({ chromium }) => chromium.launch()).then((b) => { console.log('chromium launched'); return b.close(); })")
```

## 2. Write the one file

Create `goatlite.yaml` at the root of the checkout. Four facts are required; two more are optional. Pick your own short name instead of copying the example's: this tutorial builds `pasture`, and if everyone names theirs `pasture` too, anyone who later touches two of them collides on the name everywhere it is used, from the DNS zone to the mesh profile. Name it after yourself, your team, or your field. The repository and the services a goatlite runs are read from the checkout you cloned.

```yaml
schema: goatbuilder/v1
preset: lite
lite:
  name: pasture            # the goatnet's name; its zone is pasture.lan
  host: 192.0.2.50         # the box's LAN address or hostname
  ssh_user: ubuntu         # the user your SSH key logs in as
  admin: <your-email>      # the first and only administrator: you
  zone: pasture.lan        # optional; defaults to <name>.lan
  disk_encryption: false   # optional; defaults to true. false for a VM
  data_disk: /dev/nvme0n1  # optional; the disk to encrypt, when disk_encryption is true
  ssh_pubkey: ~/.ssh/id_ed25519.pub   # optional; defaults to the first ~/.ssh/id_*.pub found
```

`disk_encryption: true` encrypts the box's data disk and asks for a passphrase at every boot. ⚠️ `data_disk` names the disk it encrypts, and encrypting the wrong disk destroys what is on it, so check the name first (**on the box over SSH:** `lsblk`). On a VM that you will delete later, set `disk_encryption` to `false`. Your SSH public key becomes the administrator key on the box. Everything else the full configuration asks for is supplied by the preset; the table of what it decides is [goatlite.md §3](../design/goatlite.md#3-what-the-preset-decides).

**A box that is not on your LAN.** goatlite is shaped for a box on the same network as your laptop. A cloud VM works for the standup itself: give its public address as `host`, open its SSH port to your address, and set `disk_encryption` to `false`. Three steps still need a machine on the box's own network, because the goatnet's edge admits only that network (it derives the allowed range from the box's address) and the names in the zone resolve only there: the browser step's hosts-file line in section 5, enrolling in section 6, and the acceptance rows in section 7 that reach the box through the mesh. A second machine beside the box, or a VM in the same network as another VM you work from, covers all three.

## 3. Rehearse it on the worked example

The worked example is a goatlite called `pasture`. Every phase runs for real against files. Nothing touches a network, and the output is the same every time. In the commands below, `--mock` points the run at the copied files instead of a box, and `--ack` signs the rehearsal's acknowledgement line; any name works.

Every command here runs on your laptop, from the root of the checkout.

Copy the example into a scratch folder, so the original stays unchanged and you can repeat this as often as you like:

```bash
WORK="$(mktemp -d)/pasture"
cp -R ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite "$WORK"
```

Run the full standup against the copy. Every phase runs for real; only the box is played by files:

```bash
goatbuilder standup pasture --mock "$WORK" --ack "A Learner" --yes
```

Read the result the way you will on a real goatnet:

```bash
goatbuilder status --env pasture --mock "$WORK"
```

List every acceptance check that applies to a one-box LAN goatnet:

```bash
goatbuilder assay pasture --values ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite/assay-values.yaml --plan
```

**Checkpoint 1.** Your output matches `ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints/00-goatlite.txt` line for line, apart from the milliseconds. Read it before moving on: the phases run in the standup's order, the internet-only phases are skipped and say so, the last line of `standup` says every declared component is verified on the one machine, and `assay --plan` lists every acceptance check that applies to a one-box LAN site. Run it twice: a second standup is always safe.

## 4. Check the laptop and the box

**On your laptop:**

```text
goatbuilder doctor
```

The doctor reads your `goatlite.yaml` and prints one `lite` row naming every decision it derived from your four facts: the box, the SSH user, the administrator, the zone, the environment and its one site, disk encryption, the secrets store. Then it names every tool it needs on the laptop, and checks that `GHCR_READ_TOKEN` is set; when it is not, the doctor prints a note, because phase 5 needs it to pull the images. The cloud checks, the browser-test check and the release checks (`gh`, `crane`, the image-digest tool, the goat contexts) show as skipped, because the lite preset declares that no cloud is used and no release is cut. Before your first standup the secrets file does not exist yet; its row passes and says phase 3 of the standup produces it. Fix what it names, then run it again until every row passes. The doctor does not reach the box; phase 1 of the standup does.

## 5. Stand it up

**On your laptop:**

```text
goatbuilder standup pasture
```

`goatbuilder` first writes the goatnet's configuration files into your checkout from your four facts, then runs the phases. What each prints, in plain words:

| Phase | What happens                                                                                                   | What you see when it worked                                                                                                   |
| ----- | -------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| 1     | Gates: the configuration files are written from your four facts, including the goatnet's access policy; the configuration is checked; every input the phases before the browser step need is set, or is named with the later phase that produces it; the box is measured, and what phase 2 will set on a fresh box is named as pending, not failed | `preflight OK`, lines saying `produced by phase 3` and `phase 4 of this run`, `OK: 'pasture' declares its ACL`, lines ending `set by phase 2 of this run`, then `READY` |
| 2     | The box is baselined; the data disk is encrypted if you asked for it                                           | the container engine's data lives on the declared disk                                                                        |
| 3     | Every secret the goatnet needs is generated into one local file                                                | the file exists, only you can read it, and git ignores it                                                                     |
| 4     | The goatnet's certificate authority is minted on the box and every service certificate is issued from it       | every certificate present                                                                                                     |
| 5     | The management stack comes up                                                                                  | on a first run this phase stops waiting for a database schema that does not exist yet. That is expected; phase 6 completes it |
| 6     | The identity service comes up, then the management stack finishes and prints a single-use setup key            | `failed=0` and the setup key, which you save                                                                                |
| 7, 8  | Skipped: internet-only                                                                                         | each says so                                                                                                                  |
| 9     | The data-fabric router and the workload-identity server come up                                                | both healthy; the router listens on the box's LAN address                                                                     |
| 10    | The LAN services come up: the dashboard, the name server for your zone, the login pages                        | the dashboard answers on the LAN                                                                                              |
| 11    | **The one browser step.** The run pauses and prints what to do, below                                    | you resume with `--from 12`                                                                                                 |
| 12    | The access policy is applied, the service identities are minted, the portal and the remaining services come up | `OK: no All-group rule; every enabled policy is declared.` then `OK: every required identity is present and verified.`    |
| 13    | The assay                                                                                                      | every applicable row `PASS`                                                                                                 |
| 14    | Skipped: no rescue node and no relay tier are declared                                                         | it says so                                                                                                                    |

A phase that fails says which phase, what state the box is in, and the resume command. Running a phase again is safe; it changes only what is missing. The repair is always the same: fix the cause, then resume from that number.

### The browser step

Phase 11 needs a person, once: the goatnet's first login creates your account, and the one service token every later identity is minted from can only be made after someone has signed in. The run prints these steps with your names filled in, and skips this pause once the token is in your secrets file.

1. **Make your laptop resolve the zone.** Service names have the form `<service>.<name>.<zone>`, so the goatnet `pasture` in the zone `pasture.lan` has its dashboard at `dashboard.pasture.pasture.lan`. The name repeats on purpose. The simplest way is one command that adds one line to the laptop's hosts file. It asks for your own password, and you replace `192.0.2.50` with your box's address. **On your laptop:**

   ```text
   echo '192.0.2.50 dashboard.pasture.pasture.lan auth.pasture.pasture.lan oauth2.pasture.pasture.lan portal.pasture.pasture.lan mgmt.pasture.pasture.lan netbird.pasture.lan' | sudo tee -a /etc/hosts
   ```

   The alternative to the hosts file is setting the laptop's DNS server to the box's address in your network settings; after section 6 the mesh takes over name resolution and you can remove the line.
2. **Read your one-time password. On the box over SSH:**

   ```text
   sudo cat /opt/mgmt-host/.bootstrap-passwords/<your-email>.txt
   ```
3. **Accept the goatnet's certificates in the browser, in this order.** Until section 8 installs the goatnet's root certificate, the browser trusts none of its pages, and it only shows the warning for pages you open yourself. The login also makes calls you cannot see, so open each of these once and accept the warning: `https://oauth2.<name>.<zone>/.well-known/openid-configuration`, `https://auth.<name>.<zone>/welcome`, and `https://mgmt.<name>.<zone>/api/users` (a small error page there is the expected answer).
4. **Sign in** at `https://dashboard.<name>.<zone>` with your email and that password.
5. **Re-run phase 10 once** to make your new account an administrator: `goatbuilder standup <name> --site <name> --from 10 --to 10`. Until you do, the dashboard refuses you.
6. **Create the service token.** In the dashboard open Team, then Service Users, and create a service user named `svc-<name>-netbird-infra` with the admin role. Create a token on it.
7. **Store the token** where the run says. Under the lite preset that is one line in the goatnet's secrets file, `<NAME>_NB_API_TOKEN=<the token>`. `goatbuilder secrets hint <NAME>_NB_API_TOKEN` names the file.

Then resume. **On your laptop:**

```text
goatbuilder standup pasture --from 12
```

## 6. Enrol your laptop

Your laptop is almost certainly on another mesh already, and this section is where people break that setup, so read the warning before any command. ⚠️ `netbird up` without `--profile` applies to whichever profile was last used. Run it bare and you enrol your existing profile into this goatnet, overwriting its login. Every command below names its profile, so none of them can touch your other mesh.

Every command here runs on your laptop. Phase 6 printed the single-use setup key this section uses.

**First, trust the goatnet's root certificate.** `netbird up` reaches the goatnet's management service over TLS, with a certificate the goatnet's own authority issued, so the laptop must trust that authority before it enrols; until then the enrolment times out. Phase 1 wrote the root into your checkout at `ops/enrollment/ca/pasture/root-cert.pem` (with your goatnet's name in place of `pasture`). Install it once.

On Ubuntu and Debian:

```text
sudo cp ops/enrollment/ca/pasture/root-cert.pem /usr/local/share/ca-certificates/pasture-root.crt
sudo update-ca-certificates
```

On Fedora and other RPM systems:

```text
sudo cp ops/enrollment/ca/pasture/root-cert.pem /etc/pki/ca-trust/source/anchors/pasture-root.pem
sudo update-ca-trust
```

On macOS:

```text
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ops/enrollment/ca/pasture/root-cert.pem
```

The NetBird service reads the trust store when it starts, and installing NetBird started it, so restart it now that the root is installed:

```text
sudo netbird service restart
```

Restart the browser too, and the warnings from the sign-in step go away. Firefox keeps its own trust store; in its settings, search for "certificates" and import the same file as an authority.

First, see your profiles. The checkmark is the profile you live on; you will return to it at the end, so note its name:

```text
netbird profile list
```

Create a profile for this goatnet and switch to it. The profile carries the goatnet's name, so a second goatnet later gets its own profile instead of overwriting this one. The name is only a label on your laptop; if you ever meet two goatnets that share a name, call the second profile anything you like, because only the management address in the enrol command decides which mesh it joins:

```text
netbird profile add pasture
netbird profile select pasture
```

Enrol, naming the profile again so this command cannot touch any other:

```text
sudo netbird up --profile pasture --management-url https://netbird.<zone>:443 --setup-key <the key from phase 6>
```

Confirm it joined:

```text
netbird status
```

### Going back to your usual mesh

Switching profiles needs no key and loses nothing; each profile keeps its own enrolment. When you finish here, select the profile that had the checkmark in the first step, and confirm you are back on it:

```text
sudo netbird up --profile <the name you noted>
netbird status
```

To visit the goatlite again later, `sudo netbird up --profile pasture` is the whole journey. `netbird profile select` on its own only switches the profile and disconnects; `up --profile` switches and connects.

You want `Management: Connected` and an address in the mesh range. From here the laptop resolves every name in your zone through the goatnet's own name server, and you can remove any hosts-file pin from the browser step.

## 7. Prove it is finished

Phase 13 ran the assay. The rows that use the `goat` command run as the goatnet's own probe identity on the box, not as you, and that identity is minted once by you, because minting key material is never a playbook's side effect. **On your laptop, once:**

```text
ops/scripts/create-assay-probe-identity.sh pasture
```

Then run the assay again, any time. **On your laptop:**

```text
goatbuilder assay pasture
```

It checks the goatnet layer by layer: the box, the management stack, TLS and DNS, the mesh, identity, the fabric, time, the service surface. Every applicable row is `PASS`. A row that could not run is `UNKNOWN`, which is never a pass, and the command exits 3 until there are none. Read the table, not the exit code.

## 8. Use it

Every command in this section runs on your laptop.

The goatnet's root certificate is already trusted: section 6 installed it before enrolment.

**Watch a message cross the fabric.** The assay's `fabric-put-subscribe-roundtrip` row publishes a value through the goatnet's router and reads it back; a `PASS` there is your goatnet carrying data. Run that row alone:

```text
goatbuilder assay pasture --only fabric-put-subscribe-roundtrip
```

**Your own identity on the goatnet.** On a larger goatnet, the identity console issues each person a welcome package. A goatlite runs no console, and you already hold its certificate authority, so you mint your own identity from it and install it as a `goat` context named after your goatnet. **On your laptop, once:**

```text
ops/scripts/create-founder-identity.sh pasture
```

It ends with `verified: goat --context pasture status opens an mTLS session to the router`. Your identity may write under `founder/`, and `goat hello` writes under your goatnet's name.

**Read its health.**

```text
goat --context pasture status
goat --context pasture health
```

`goat status` opens one real session to the router and gives one verdict per layer. `goat health` reads the fabric's own health beat.

**Send a message to yourself.** `goat hello` publishes through the fabric and receives its own message back.

```text
goat --context pasture hello
```

**Publish and subscribe for real.** Subscribe in one terminal, publish in another, and watch the message arrive.

```text
goat --context pasture sub 'founder/**' --take 1 --for 60s
```

```text
goat --context pasture pub founder/hello/v1 --payload 'first message on pasture' --confirm
```

`--confirm` reads the value back from the router's storage, so a denied write is reported instead of silently dropped.

## When something fails

Every command in this section runs on your laptop. Every error `goat` and `goatbuilder` print says what happened, the likely cause, what to do and where to read. Paste any error, or its code, back in:

```text
goatbuilder explain GB-029
goatbuilder explain "Permission denied (publickey)"
goat explain E_OFF_MESH
```

For the goatnet as a whole, run the assay and read the first row that is not `PASS`:

```text
goatbuilder assay pasture
```

When the catalog does not know your error, `goatbuilder explain --report -` writes the text and its interpretation to a file to share, so the catalog grows.

## Undo

Re-running is the repair: fix the cause and resume from the failed phase. `goatbuilder teardown pasture --site pasture --dry-run` prints the teardown order, which is the standup in reverse; `--services` stops the services and keeps every volume.

## What you have

A real goatnet on one box: its own mesh with your laptop on it, its own identity service with you as its administrator, its own certificate authority, its own data-fabric router carrying your messages, every secret in one local file, and no account with any outside service beyond the image-pull token. What you learn on it applies to every larger goatnet. The growth path to an internet-enrollable goatnet is an `internet:` block added to the `lite:` entry and a re-run, never a second standup; the renderer writes the internet surface from it, and its status (built, not yet stood up against real relays) is in [goatlite.md §6](../design/goatlite.md#6-growing-out-of-lite).

The next tutorial is [Tutorial 1](01-new-goatnet.md), the full shape: the same fourteen phases with every decision this preset made for you written out, a founder site that can take an observability host, a rescue node and, later, federated nodes.
