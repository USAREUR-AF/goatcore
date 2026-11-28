# Your name and mailbox in public commit history

Every commit records an author address, a committer address and, with `git commit -s`, a `Signed-off-by:` line. All three are public and permanent once pushed. You can contribute with signed, verified commits without publishing a personal mailbox. This page says how, and what the `commit-identity` check does when a pull request carries one.

## Set it up once

1. **Find your GitHub no-reply address.** On GitHub: Settings, then Emails. With "Keep my email addresses private" switched on, the page shows an address of the form `12345678+octocat@users.noreply.github.com`.
2. **Switch on two settings on that page.**
   - "Keep my email addresses private": commits you make on github.com (merges, web edits) use the no-reply address.
   - "Block command line pushes that expose my email": GitHub refuses a push from the command line whose commits expose your private address.
3. **Tell git to use the no-reply address.**

   ```sh
   git config --global user.name  "octocat"
   git config --global user.email "12345678+octocat@users.noreply.github.com"
   ```

   If you use another address for other work, set it for this organization's clones only. In `~/.gitconfig`:

   ```ini
   [includeIf "hasconfig:remote.*.url:https://github.com/USAREUR-AF/**"]
       path = ~/.gitconfig-goat
   ```

   and put the `[user]` name and email in `~/.gitconfig-goat`. Add a second block with the SSH form of the URL if you clone over SSH.

   The sign-off line is built from the same two values, so this also fixes `git commit -s`. The sign-off is where a personal mailbox most often enters a history: GitHub's push block does not read the commit message.
4. **Keep signing.** A signature is verified when the key is registered on your GitHub account as a signing key and the committer address is one of your account's addresses. The no-reply address is one of them.

   ```sh
   git config --global gpg.format ssh
   git config --global user.signingkey ~/.ssh/id_ed25519.pub
   git config --global commit.gpgsign true
   ```

The name can be a handle. The Developer Certificate of Origin asks you to certify the contribution under an identity you stand behind and use consistently; it does not ask for a mailbox that receives mail.

## Check before you push

```sh
git config user.name; git config user.email                # what the next commit will carry
git log -1 --format='author:    %an <%ae>%ncommitter: %cn <%ce>%n%(trailers:key=Signed-off-by,key=Co-authored-by)'
```

## What the check does

`commit-identity` reads every commit a pull request adds: the author address, the committer address, and each `Signed-off-by:` and `Co-authored-by:` line. An address passes when it is a GitHub no-reply address, an address GitHub itself writes, or an address under a domain reserved for examples. Any other address is treated as a personal mailbox.

The first time one of your pull requests carries your personal mailbox, the check fails and comments once on the pull request, showing the address masked. You then choose:

- **Keep it out.** Set the no-reply address as above, then rewrite the branch. The comment gives the exact lines; they are:

  ```sh
  git rebase --exec 'git log -1 --format=%B | grep -v "^Signed-off-by:" | git commit --amend --reset-author -S -s -F -' origin/main
  git push --force-with-lease
  ```

  This re-makes each commit as you, signed, with a fresh sign-off. Use it only on a branch whose commits are all yours.
- **Accept it.** Comment exactly `/email-ok` on the pull request. The check turns green without a new push; if it has not after a minute, close and reopen the pull request. It is remembered for your own address on every later pull request in every repository of the organization, and no later pull request of yours is stopped or commented on for it. To take it back, delete that comment.

Only the person an address belongs to can accept it. The check knows whose it is when GitHub maps the commit to an account, or when a sign-off carries the commit author's name or login. An address with no known owner, such as a co-author line for someone else, cannot be accepted by another person: it is fixed, or an owner of the repository comments `/email-ok-maintainer`, which holds for that one pull request.

The check stores no mailbox. The record is the `/email-ok` comment you wrote. A workflow keeps an index of where each person's comment is, as one comment on an issue of the organization's `.github` repository, so the check in another repository finds it; the index lists user names and links, and an entry goes when its comment is deleted. In a private repository the acceptance holds for that repository.

The check reads only the commits a pull request adds. It does not read or change commits already merged.

## What this does not change

Commits already pushed keep what they carry. The steps above apply to every commit you make from now on.

The decision is [ADR 1276](../adr/1276-a-personal-mailbox-in-a-pull-request-is-refused-once.md).
