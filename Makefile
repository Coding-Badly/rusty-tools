
AMI_HELPER=/usr/local/bin/ami-helper

${AMI_HELPER}:
	curl --location https://github.com/Coding-Badly/rusty-tools/releases/download/current/ami-helper --output ami-helper-VkVNU8nd
	chmod u+x+r-w,g=,o= ami-helper-VkVNU8nd
	sudo mv ami-helper-VkVNU8nd /usr/local/bin/ami-helper

smoke-test-ubuntu: ${AMI_HELPER}
	ami-helper version

