// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <linux/vm_sockets.h>
#include <unistd.h>
#include <string.h>

#define PORT 1234

int main()
{
	int sock;
	char *hello = "echo 'Hello from host'\n";
	char buffer[1024] = { 0 };
	struct sockaddr_vm serv_addr;

	if ((sock = socket(AF_VSOCK, SOCK_STREAM, 0)) < 0) {
		printf("\n Socket creation error\n");
		return -1;
	}
	printf("\n Create socket successfully!\n");
	serv_addr.svm_family = AF_VSOCK;
	serv_addr.svm_cid = VMADDR_CID_HOST;
	serv_addr.svm_port = PORT;

	if (connect(sock, (struct sockaddr *)&serv_addr, sizeof(serv_addr)) <
	    0) {
		printf("\nConnection Failed \n");
		return -1;
	}
	printf("\n Socket connect successfully!\n");

	// Send message to the server and receive the reply
	if (send(sock, hello, strlen(hello), 0) < 0) {
		printf("\nSend Failed\n");
		return -1;
	}
	printf("Hello message sent\n");
	if (read(sock, buffer, 1024) < 0) {
		printf("\nRead Failed\n");
		return -1;
	}
	printf("Server: %s\n", buffer);
	return 0;
}