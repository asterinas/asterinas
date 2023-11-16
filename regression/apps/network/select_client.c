#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>

#define DEFAULT_PORT 4564

int main(int argc, char *argv[]) {
	int connfd = 0;
	int cLen = 0;
	struct sockaddr_in client;
	

	client.sin_family = AF_INET;
	client.sin_port = htons(DEFAULT_PORT);
	client.sin_addr.s_addr = inet_addr("127.0.0.1");

	connfd = socket(AF_INET, SOCK_STREAM, 0);

	if(connfd < 0) {
		perror("socket");
		return -2;
	}

	if(connect(connfd, (struct sockaddr*)&client, sizeof(client)) < 0) {
		perror("connect");
		return -3;
	}

	char buffer[1024];
	bzero(buffer, sizeof(buffer));
	recv(connfd, buffer, 1024, 0);
	printf("recv: %s\n", buffer);
	bzero(buffer, sizeof(buffer));
	strcpy(buffer, "this is client!\n");
	send(connfd, buffer, 1024, 0);
	while(1) {
		bzero(buffer, sizeof(buffer));
		scanf("%s", buffer);
		int p = strlen(buffer);
		buffer[p] = '\0';
		send(connfd, buffer, 1024, 0);
		printf("I have send buffer\n");
	}
	close(connfd);
	return 0;
}