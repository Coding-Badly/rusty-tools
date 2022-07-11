# rusty-tools

Some handy tools built using Rust.

## ami-helper

The `ami-helper` is a Linux / Windows command line program that tries to determine the most recent
AMI for Amazon Linux, Debian, and Ubuntu.  It's designed to be used in code that starts EC2
instances like our Agent Smoke Tests.

### Install

Install `ami-helper` using `wget`.

``` bash
wget https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper
chmod u+x+r-w,g=,o= ami-helper
./ami-helper

```

Install `ami-helper` using `curl`.

``` bash
curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper --output ami-helper
chmod u+x+r-w,g=,o= ami-helper
./ami-helper

```

Install `ami-helper` on Windows (assuming `curl` is available).

``` bash
curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper.exe --output ami-helper.exe

```

### Use

``` bash
USE_THIS_AMI=$(./ami-helper select --operating-system amazon --architecture amd64 --just-ami --singleton)
printf "> %s <\n" $USE_THIS_AMI

```

### Cleanup

``` bash
rm -f ./ami-helper

```
