// SPDX-License-Identifier: MPL-2.0

#include "mongoose.h"

static const char *s_url = "http://127.0.0.1:8080/";
static const char *s_post_data = NULL; // POST data

// Print HTTP response and signal that we're done
static void fn(struct mg_connection *c, int ev, void *ev_data)
{
	if (ev == MG_EV_CONNECT) {
		// Connected to server. Send request
		struct mg_str host = mg_url_host(s_url);
		int content_length = s_post_data ? (int)strlen(s_post_data) : 0;

		mg_printf(c,
			  "%s %s HTTP/1.1\r\n"
			  "Host: %.*s\r\n"
			  "Content-Length: %d\r\n"
			  "\r\n",
			  s_post_data ? "POST" : "GET", mg_url_uri(s_url),
			  (int)host.len, host.ptr, content_length);

		mg_send(c, s_post_data, content_length);
	} else if (ev == MG_EV_HTTP_MSG) {
		// Response is received. Print it
		struct mg_http_message *hm = (struct mg_http_message *)ev_data;
		printf("%.*s", (int)hm->message.len, hm->message.ptr);
		c->is_draining = 1; // Tell mongoose to close this connection
		*(bool *)c->fn_data = true; // Tell event loop to stop
	} else if (ev == MG_EV_ERROR) {
		*(bool *)c->fn_data = true; // Error, tell event loop to stop
	}
}

int main(int argc, char *argv[])
{
	struct mg_mgr mgr;
	if (argc > 1)
		s_url = argv[1]; // Use URL provided on the command line
	bool done = false;

	mg_mgr_init(&mgr); // Initialize event manager

	mg_http_connect(&mgr, s_url, fn, &done); // Create client connection
	while (!done)
		mg_mgr_poll(&mgr, 50);

	mg_mgr_free(&mgr); // Free resources
	return 0;
}
