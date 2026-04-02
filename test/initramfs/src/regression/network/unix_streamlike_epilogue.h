/* SPDX-License-Identifier: MPL-2.0 */

FN_SETUP(cleanup)
{
	CHECK(close(sk_unbound));

	CHECK(close(sk_bound));

	CHECK(close(sk_listen));

	CHECK(close(sk_connected));

	CHECK(close(sk_accepted));

	CHECK(unlink(BOUND_ADDR.sun_path));

	CHECK(unlink(LISTEN_ADDR.sun_path));
}
END_SETUP()
