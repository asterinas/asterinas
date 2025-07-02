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
	char buffer[1024] = { 0 };
	FILE *commandFile;
	struct sockaddr_vm serv_addr;

	commandFile = fopen("../vsock_commands.sh", "r");
	if (commandFile == NULL) {
		perror("Failed to open the command file");
		return -1;
	}

	if ((sock = socket(AF_VSOCK, SOCK_STREAM, 0)) < 0) {
		perror("\n Socket creation error");
		fclose(commandFile);
		return -1;
	}
	printf("\n Create socket successfully!\n");

	serv_addr.svm_family = AF_VSOCK;
	serv_addr.svm_cid = VMADDR_CID_HOST;
	serv_addr.svm_port = PORT;

	if (connect(sock, (struct sockaddr *)&serv_addr, sizeof(serv_addr)) <
	    0) {
		perror("\nConnection Failed");
		close(sock);
		fclose(commandFile);
		return -1;
	}
	printf("\n Socket connected successfully!\n");

	char command[1024];
	while (fgets(command, sizeof(command), commandFile) != NULL) {
		if (send(sock, command, strlen(command), 0) < 0) {
			perror("\nSend Failed");
			break;
		}
		printf("Command sent: %s", command);
		memset(buffer, 0, sizeof(buffer));
		if (read(sock, buffer, sizeof(buffer)) < 0) {
			perror("\nRead Failed");
			break;
		}
		printf("Server: %s\n", buffer);
	}

	close(sock);
	fclose(commandFile);

	return 0;
}
