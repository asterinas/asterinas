#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

static int new_bound_socket(struct sockaddr_in *addr)
{
    int sockfd;

    sockfd = socket(PF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) {
        perror("new_bound_socket: socket");
        return -1;
    }

    if (bind(sockfd, (struct sockaddr *)addr, sizeof(*addr)) < 0) {
        perror("new_bound_socket: bind");
        close(sockfd);
        return -1;
    }

    return sockfd;
}

static int new_connected_socket(struct sockaddr_in *addr)
{
    int sockfd;

    sockfd = socket(PF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) {
        perror("new_connected_socket: socket");
        return -1;
    }

    if (connect(sockfd, (struct sockaddr *)addr, sizeof(*addr)) < 0) {
        perror("new_connected_socket: connect");
        close(sockfd);
        return -1;
    }

    return sockfd;
}

#define MAX_TEST_BACKLOG 5

int test_listen_backlog(struct sockaddr_in *addr, int backlog)
{
    int listenfd;
    int connectfd[MAX_TEST_BACKLOG], acceptfd[MAX_TEST_BACKLOG];
    int num_connectfd = 0, num_acceptfd = 0;
    int ret = -1;
    struct sockaddr caddr;
    socklen_t caddrlen = sizeof(caddr);

    listenfd = new_bound_socket(addr);
    if (listenfd < 0) {
        fprintf(stderr,
            "Test failed: Error occurs in new_bound_socket\n");
        return -1;
    }

    if (listen(listenfd, backlog) < 0) {
        perror("listen");
        fprintf(stderr, "Test failed: Error occurs in listen\n");
        goto out;
    }

    for (; num_connectfd < backlog; ++num_connectfd) {
        connectfd[num_connectfd] = new_connected_socket(addr);
        if (connectfd[num_connectfd] < 0)
            break;
    }

    if (num_connectfd != backlog) {
        fprintf(stderr,
            "Test failed: listen(backlog=%d) allows only %d pending connections\n",
            backlog, num_connectfd);
        goto out;
    }
    fprintf(stderr,
        "Test passed: listen(backlog=%d) allows >=%d pending connections\n",
        backlog, num_connectfd);

    for (; num_acceptfd < num_connectfd; ++num_acceptfd) {
        acceptfd[num_acceptfd] = accept(listenfd, &caddr, &caddrlen);
        if (acceptfd[num_acceptfd] < 0) {
            perror("accept");
            break;
        }
    }

    if (num_acceptfd != num_connectfd) {
        fprintf(stderr,
            "Test failed: Only %d pending connections can be accept()'ed out "
            "of %d\n",
            num_acceptfd, num_connectfd);
        goto out;
    }
    fprintf(stderr,
        "Test passed: All of %d pending connections can be accept()'ed\n",
        num_acceptfd);

    ret = 0;

out:
    while (--num_acceptfd >= 0)
        close(acceptfd[num_acceptfd]);

    while (--num_connectfd >= 0)
        close(connectfd[num_connectfd]);

    close(listenfd);

    return ret;
}

int main(void)
{
    struct sockaddr_in addr;
    int backlog;
    int err = 0;

    addr.sin_family = AF_INET;
    if (inet_aton("127.0.0.1", &addr.sin_addr) < 0) {
        fprintf(stderr, "inet_aton cannot parse 127.0.0.1\n");
        return -1;
    }

    for (backlog = 0; backlog <= MAX_TEST_BACKLOG; ++backlog) {
        // Avoid "bind: Address already in use"
        addr.sin_port = htons(8080 + backlog);

        err = test_listen_backlog(&addr, backlog);
        if (err != 0)
            break;
    }

    return err ? -1 : 0;
}
