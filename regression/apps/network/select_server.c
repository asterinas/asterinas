#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <pthread.h>
#include <sys/select.h>
#include <sys/time.h>
#include <time.h>

#define LOG_FILE_NAME "2d2.txt"
FILE *filedis = NULL;

#define PORT 4564

long long factorial(long long n){

	unsigned long long ans = 1;
	for (int i = 1 ; i <= n ; i++){
		ans *= i;
	}
	return ans;

}

int check(int exp, const char* msg){
	if( exp < 0){
		perror(msg);
		exit(1);
	}
}


int main(){

	int sockfd, b, newSocket;

	struct sockaddr_in serverAddr, clienAddr;

	socklen_t addr_size;

	char mssg[100];   
	pid_t pid;

	sockfd = socket(AF_INET, SOCK_STREAM, 0);
	check(sockfd, "error in socket\n");


	memset(&serverAddr, '\0', sizeof(serverAddr));
	serverAddr.sin_addr.s_addr = inet_addr("127.0.0.1");
	serverAddr.sin_family = AF_INET;
	serverAddr.sin_port = htons(PORT);


	b = bind(sockfd, (struct sockaddr*)&serverAddr, sizeof(serverAddr));
	check(b, "error in bind\n");

	if(listen(sockfd, 10) < 0)perror("Error on listening\n");


    fd_set fds, readfds;
    FD_ZERO(&fds);
    FD_SET(sockfd, &fds);
    if( filedis == NULL)filedis = fopen( LOG_FILE_NAME, "w");
    int fdmax = sockfd;


	double time_spent = 0.0;
	clock_t begin = clock();


    while(1){


        readfds = fds;
        if( syscall(23,fdmax + 1 , &readfds, NULL, NULL, NULL) < 0) perror("error at select");
        
        for( int fd = 0; fd < (fdmax + 1); fd++){
            
            if( FD_ISSET( fd, &readfds)){  // check if this fd is ready for reading

                if( fd == sockfd){    // request for new connection

                    newSocket = accept(sockfd, (struct sockaddr*)&clienAddr, &addr_size);
                    if(newSocket < 0){
                        exit(1);
                    }

                    char *IP = inet_ntoa(clienAddr.sin_addr);
                    int PORT_NO = ntohs(clienAddr.sin_port);
                    

                    fprintf(filedis, "IP : %s  PORT : %d\n", IP, PORT_NO);
                    
                    printf("Connection accepted from IP : %s: and PORT : %d\n", IP, PORT_NO);

                    FD_SET(newSocket, &fds);
                    if( newSocket > fdmax)fdmax = newSocket;


                }else{   // some client is sending data

                    bzero(mssg, 100);
                    int numbytes = recv( fd, &mssg, sizeof(mssg), 0);

                    long long num = atoi(mssg);
                    fprintf(filedis, "INTEGER : %lld  FACTORIAL : %lld\n", num , factorial(num));
                    sprintf(mssg, "%lld", factorial(num));

                    send(fd, &mssg, sizeof(mssg), 0);

                }

            }
        }

    }
    
    
    clock_t end = clock();
	time_spent += (double)(end - begin) / CLOCKS_PER_SEC;
    printf("The elapsed time is %f seconds", time_spent);

    close(sockfd);

	return 0;
}