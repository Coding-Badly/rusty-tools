
ami-helper:
	wget https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper
	sudo mv ami-help /usr/local/bin/ami-helper
	chmod u+x+r-w,g=,o= /usr/local/bin/ami-helper

smoke-test-ubuntu: ami-helper
	ami-helper version

