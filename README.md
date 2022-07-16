# rusty-tools

Some handy tools built using Rust.

## ami-helper

`ami-helper` is a Linux / macOS / Windows command line program that tries to determine the most
recent AMI for Amazon Linux, Debian, Ubuntu, and Windows.  It's designed to be used in code that
starts EC2 instances like our Agent Smoke Tests.

### Install

Install `ami-helper` for Linux AMD64 using `wget`.

``` bash
wget https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper.linux.amd64
chmod u+x+r-w,g=,o= ami-helper.linux.amd64
sudo mv -f ami-helper.linux.amd64 /usr/local/bin/ami-helper
ami-helper version

```

Install `ami-helper` for Linux AMD64 using `curl`.

``` bash
curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper.linux.amd64 --output ami-helper-VkVNU8nd
chmod u+x+r-w,g=,o= ami-helper-VkVNU8nd
sudo mv -f ami-helper-VkVNU8nd /usr/local/bin/ami-helper
ami-helper version

```

Install `ami-helper` for macOS AMD64 using `curl`.

``` bash
curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper.macos.amd64 --output ami-helper-zFzB8WTL
chmod u+x+r-w,g=,o= ami-helper-zFzB8WTL
sudo mv -f ami-helper-zFzB8WTL /usr/local/bin/ami-helper
ami-helper version

```

Install `ami-helper` for Windows (assuming `curl` is available).

``` bash
curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper.windows.amd64 --output ami-helper.exe
ami-helper version

```

### Examples

``` bash
USE_THIS_AMI=$(ami-helper select --operating-system amazon --architecture amd64 --just-ami --singleton)
printf "> %s <\n" $USE_THIS_AMI

```

``` bash
START_OPTIONS=$(ami-helper select -o ubuntu -a arm64 -r us-west-1 -s)
printf "> %s <\n" $START_OPTIONS

```

### Cleanup

``` bash
sudo rm -f /usr/local/bin/ami-helper

```

``` bash
rm -f ami-helper

```
