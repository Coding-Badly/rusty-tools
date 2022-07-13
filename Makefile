
AMI_HELPER=/usr/local/bin/ami-helper

${AMI_HELPER}: 
	wget https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper
	sudo mv ami-help /usr/local/bin/ami-helper
	chmod u+x+r-w,g=,o= /usr/local/bin/ami-helper

smoke-test-ubuntu: ${AMI_HELPER}
	ami-helper version

