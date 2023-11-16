#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>

#define IPADDRESS   "127.0.0.1"
#define PORT        9000
#define MAXSIZE     100 
#define LISTENQ     5
#define FDSIZE      1000
#define EPOLLEVENTS 100

static int socket_bind(const char* ip, int port);
static void do_epoll(int listenfd);
static void handle_events(int epollfd, struct epoll_event *events,
                          int num, int listenfd, char *buf);
static void handle_accpet(int epollfd, int listenfd);
static void do_read(int epollfd, int fd, char *buf);
static void do_write(int epollfd, int fd, char *buf);
static void add_event(int epollfd, int fd, int state);
static void modify_event(int epollfd, int fd, int state);
static void delete_event(int epollfd, int fd, int state);

int main(int argc,char *argv[]) {
    int  listenfd;
    listenfd = socket_bind(IPADDRESS,PORT);
        printf("listenfd %d\n", listenfd);

    listen(listenfd,LISTENQ);
  
    printf("do epoll2\n");
    do_epoll(listenfd);
    return 0;
}

static int socket_bind(const char* ip,int port) {
    int  listenfd;
    struct sockaddr_in servaddr;
    listenfd = socket(AF_INET,SOCK_STREAM,0);
    if (listenfd == -1) {
        perror("socket error:");
        exit(1);
    }
    bzero(&servaddr, sizeof(servaddr));
    servaddr.sin_family = AF_INET;
    inet_pton(AF_INET, ip, &servaddr.sin_addr);
    servaddr.sin_port = htons(port);
    if (bind(listenfd, (struct sockaddr*)&servaddr, sizeof(servaddr)) == -1) {
        perror("bind error: ");
        exit(1);
    }
    return listenfd;
}

static void do_epoll(int listenfd) {
    printf("begin do_epoll\n");
    int epollfd;
    struct epoll_event events[EPOLLEVENTS];
    int ready_cnt;
    char buf[MAXSIZE];

    printf("mem set\n");
    memset(buf, 0, MAXSIZE);
    printf("epoll create");
    epollfd = epoll_create(FDSIZE);
    printf("add event\n");

    add_event(epollfd, listenfd, EPOLLIN);
    for ( ; ; ) {
        printf("epoll wait\n");

        ready_cnt = epoll_wait(epollfd, events, EPOLLEVENTS, -1);
        handle_events(epollfd, events, ready_cnt, listenfd, buf);
    }
    close(epollfd);
}

static void
handle_events(int epollfd, struct epoll_event *events, int num,
              int listenfd, char *buf) {
    int i;
    int fd;

    for (i = 0; i < num; i++) {
        fd = events[i].data.fd;
        // If fd is a listen fd, we do accept(), otherwise it is a
        // connected fd, we should read buf if EPOLLIN occured.
        if ((fd == listenfd) && (events[i].events & EPOLLIN))
            handle_accpet(epollfd, listenfd);
        else if (events[i].events & EPOLLIN)
            do_read(epollfd, fd, buf);
        else if (events[i].events & EPOLLOUT)
            do_write(epollfd, fd, buf);
    }
}
static void handle_accpet(int epollfd,int listenfd) {
    int clifd;
    struct sockaddr_in cliaddr;
    socklen_t  cliaddrlen;

    clifd = accept(listenfd, (struct sockaddr*) &cliaddr, &cliaddrlen);
    if (clifd == -1)
        perror("Accpet error:");
    else {
        printf("Accept a new client: %s:%d\n",
               inet_ntoa(cliaddr.sin_addr), cliaddr.sin_port);
        add_event(epollfd, clifd, EPOLLIN);
    }
}

static void do_read(int epollfd, int fd, char *buf) {
    int nread;

    nread = read(fd, buf, MAXSIZE);
    if (nread == -1) {
        perror("Read error:");
        delete_event(epollfd, fd, EPOLLIN);
        close(fd);
    }
    else if (nread == 0) {
        fprintf(stderr, "Client closed.\n");
        delete_event(epollfd, fd, EPOLLIN);
        close(fd);
    }
    else {
        //printf("Read message is : %s", buf);
        modify_event(epollfd, fd, EPOLLOUT);
    }
}

static void do_write(int epollfd, int fd, char *buf) {
    int nwrite;

    nwrite = write(fd, buf, strlen(buf));
    if (nwrite == -1) {
        perror("Write error:");
        delete_event(epollfd, fd, EPOLLOUT);
        close(fd);
    }
    else
        modify_event(epollfd, fd, EPOLLIN);

    memset(buf, 0, MAXSIZE);
}

static void add_event(int epollfd, int fd, int state) {
    struct epoll_event ev;
    ev.events = state;
    ev.data.fd = fd;
    if (epoll_ctl(epollfd, EPOLL_CTL_ADD, fd, &ev) < 0) {
        printf("Add event failed!\n");
    }
}

static void delete_event(int epollfd,int fd,int state) {
    struct epoll_event ev;
    ev.events = state;
    ev.data.fd = fd;
    if (epoll_ctl(epollfd, EPOLL_CTL_DEL, fd, &ev) < 0) {
        printf("Delete event failed!\n");
    }
}

static void modify_event(int epollfd,int fd,int state) {
    struct epoll_event ev;
    ev.events = state;
    ev.data.fd = fd;
    if (epoll_ctl(epollfd, EPOLL_CTL_MOD, fd, &ev) < 0) {
        printf("Modify event failed!\n");
    }
}