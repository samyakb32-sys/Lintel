# Security

## Supported versions

Security fixes go into the latest release. Please check that you can reproduce the problem on it before you report.

## Report a vulnerability

Please report privately. Do not open a public issue for a vulnerability.

Use GitHub's private reporting on this repository: open the **Security** tab and choose **Report a vulnerability**, or go straight to the [private report form](../../security/advisories/new).

A good report holds:

- the version of METEOR V1 and of Windows;
- what an attacker can do, and what they need first (for example another program running as the same user);
- the steps to reproduce it, with a small example if you have one.

Never put a real key, token or personal file in a report.

You will get an answer in the advisory. Please give us a reasonable time to fix the problem before you publish details. You are credited in the release notes unless you prefer not to be.

## What is in scope

METEOR V1 runs on the user's own computer and has no server. The parts that matter most:

- **The relay** (`lintel-hook.exe`) and the named pipe between it and the app. For example: another user or program reading or answering agent events, the relay talking to a process that is not the app, or the relay starting a program it should not.
- **The permission cards.** Anything that approves or denies an agent's request without the user's own click, or that ties a click to a different request than the one the card showed.
- **The hardware helper** (`lintel-hw.exe`), the only part that runs as administrator. For example: another process using its pipe, or making it do anything beyond reading sensors and setting fans within its limits.

Also in scope:

- a key or token written to disk, to the log or handed to a web view;
- a change to an agent's settings file without backup, diff and confirmation;
- a network call to a host the user did not turn on;
- a downloaded model or program that is used without its SHA-256 check;
- anything that sends, deletes or runs something without the user's confirmation.

## What is out of scope

- An attacker who already has administrator rights, or full control of the user's account.
- Faults in an agent program, in a model provider, in Windows or in the WebView2 runtime. Please report those to their owners.
- The installer not being code-signed. This is known and listed in the README.
