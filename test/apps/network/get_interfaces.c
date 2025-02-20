#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>

#define BUFFER_SIZE 4096

struct nl_req {
	struct nlmsghdr hdr;
	struct rtgenmsg gen;
};

int main()
{
	int sock_fd;
	struct sockaddr_nl sa;
	char buffer[BUFFER_SIZE];

	// 创建Netlink socket
	sock_fd = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
	if (sock_fd < 0) {
		perror("socket creation failed");
		exit(EXIT_FAILURE);
	}

	memset(&sa, 0, sizeof(sa));
	sa.nl_family = AF_NETLINK;

	// 绑定socket
	if (bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
		perror("bind failed");
		close(sock_fd);
		exit(EXIT_FAILURE);
	}

	// 构造请求消息
	struct nl_req req;
	memset(&req, 0, sizeof(req));
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	req.hdr.nlmsg_type = RTM_GETLINK;
	req.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
	req.hdr.nlmsg_seq = 1;
	req.gen.rtgen_family = AF_PACKET;

	// 发送请求
	struct iovec iov = { &req, req.hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	if (sendmsg(sock_fd, &msg, 0) < 0) {
		perror("sendmsg failed");
		close(sock_fd);
		exit(EXIT_FAILURE);
	}

	// 接收响应
	ssize_t len;
	while ((len = recv(sock_fd, buffer, BUFFER_SIZE, 0)) > 0) {
		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		// 解析每个消息
		for (; NLMSG_OK(nlh, len); nlh = NLMSG_NEXT(nlh, len)) {
			if (nlh->nlmsg_type == NLMSG_DONE) {
				close(sock_fd);
				exit(0);
			}

			if (nlh->nlmsg_type == NLMSG_ERROR) {
				perror("netlink error");
				break;
			}

			if (nlh->nlmsg_type == RTM_NEWLINK) {
				struct ifinfomsg *ifi = NLMSG_DATA(nlh);
				struct rtattr *attr = IFLA_RTA(ifi);
				int remaining = nlh->nlmsg_len -
						NLMSG_LENGTH(sizeof(*ifi));

				// 遍历属性
				for (; RTA_OK(attr, remaining);
				     attr = RTA_NEXT(attr, remaining)) {
					if (attr->rta_type == IFLA_IFNAME) {
						printf("Interface: %s\n",
						       (char *)RTA_DATA(attr));
						break;
					}
				}
			}
		}
	}

	close(sock_fd);
	return 0;
}
