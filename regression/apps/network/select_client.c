#include <unistd.h>
#include <stdio.h>
#include <sys/socket.h>
#include <string.h>
#include <stdlib.h>
#include <netinet/in.h>
#include <string.h>
#include <sys/types.h>
#include <netdb.h>
#include <arpa/inet.h>

#define PORT 4444

int main()
{
	
	int sockfd, ret, n;
	struct sockaddr_in serverAddr;
	char buffer[100];

	sockfd = socket(AF_INET, SOCK_STREAM, 0);
	if(sockfd < 0) perror("error in socket\n");


	memset(&serverAddr, '\0', sizeof(serverAddr));
	serverAddr.sin_family = AF_INET;
	serverAddr.sin_port = htons(PORT);
	serverAddr.sin_addr.s_addr = inet_addr("127.0.0.1");


	ret = connect(sockfd, (struct sockaddr*)&serverAddr, sizeof(serverAddr));
	if(ret < 0) perror("error in connect\n");


	int requests = 20;

	while(requests -- > 0)
	{

		bzero(buffer, 100);
		
		sprintf(buffer, "%d", requests+1);
		n = send(sockfd, &buffer, strlen(buffer), 0);

		bzero(buffer, 100);
		// n = recv(sockfd, &buffer, 100, 0);
		n = read(sockfd, &buffer, 100);

		printf("Factorial returned by Server : %s\n\n", buffer);

	}
	
	close(sockfd);

	return 0;
}