
AMI_HELPER=/usr/local/bin/ami-helper

${AMI_HELPER}: 
	curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper --output ami-helper
	chmod u+x+r-w,g=,o= ami-helper
	sudo mv ami-helper /usr/local/bin

smoke-test-ubuntu: ${AMI_HELPER}
	ami-helper version

