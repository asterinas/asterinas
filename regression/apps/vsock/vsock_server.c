// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <linux/vm_sockets.h>
#include <unistd.h>
#include <string.h>

#define PORT 4321

int main()
{
	int sock, new_sock;
	char *hello = "Hello from asterinas";
	char buffer[1024] = { 0 };
	struct sockaddr_vm serv_addr, client_addr;
	int addrlen = sizeof(client_addr);

	if ((sock = socket(AF_VSOCK, SOCK_STREAM, 0)) < 0) {
		printf("\n Socket creation error\n");
		return -1;
	}
	printf("\nCreate socket successfully\n");

	serv_addr.svm_family = AF_VSOCK;
	serv_addr.svm_cid = VMADDR_CID_ANY;
	serv_addr.svm_port = PORT;

	if (bind(sock, (struct sockaddr *)&serv_addr, sizeof(serv_addr)) < 0) {
		printf("\nBind Failed \n");
		return -1;
	}
	printf("\nBind socket successfully\n");

	if (listen(sock, 3) < 0) {
		printf("\nListen Failed\n");
		return -1;
	}
	printf("\nListen socket successfully\n");

	if ((new_sock = accept(sock, (struct sockaddr *)&client_addr,
			       (socklen_t *)&addrlen)) < 0) {
		printf("\nAccept Failed\n");
		return -1;
	}
	printf("\nAccept socket successfully\n");

	// Send message to the server and receive the reply
	if (read(new_sock, buffer, 1024) < 0) {
		printf("\nRead Failed\n");
		return -1;
	}
	printf("Client: %s\n", buffer);
	if (send(new_sock, hello, strlen(hello), 0) < 0) {
		printf("\nSend Failed\n");
		return -1;
	}
	printf("Hello message sent\n");
	return 0;
}