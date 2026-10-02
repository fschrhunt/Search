# Security

search is built to run on a private machine and be reached over a private
network. It still treats every URL and every page as hostile.

## The fetcher

A URL is model-chosen, so the fetcher assumes it is hostile:

- **Private destinations are refused.** Loopback, RFC1918, link-local (including
  cloud metadata at `169.254.169.254`), CGNAT, multicast, and IPv6 unique-local
  are refused by literal, by name, and — through a resolver installed on the
  client — by every address a name resolves to. A DNS answer that changes between
  the check and the dial (rebinding) is caught, because the socket opens to the
  address that passed the guard.
- **Every IPv6 form that embeds such an IPv4 is refused**, including NAT64
  (`64:ff9b::/96`), 6to4 (`2002::/16`), and IPv4-compatible (`::/96`), and the
  bracketed forms `url` produces.
- **Redirects are re-checked per hop**, bounded by `max_redirects`. A
  client-side redirect (a meta refresh or a scripted location change) is
  followed only through the same guard, so page content cannot steer the fetcher
  inside the network.
- **Bodies are size-capped** and **requests carry a deadline**.

The one escape hatch, `allow_private`, disables the guard for tests and
air-gapped mirrors. Never set it on a reachable service.

## Untrusted content

A fetched page is untrusted content, and the cheapest place to hide an
instruction aimed at a model is text a person never sees. Extraction removes
visually hidden text — inline `display:none`/`visibility:hidden`/`opacity:0`,
the `hidden` attribute, `aria-hidden`, screen-reader class names, and off-screen
positioning — and neutralizes links and images, so page content cannot form a
markdown image that exfiltrates. The remaining text is passed to the model as
data, in a clearly delimited field.

The strongest defense is architectural: search reads pages and returns text. It
holds no private data and cannot itself send anything onward, so the "lethal
trifecta" of private data, untrusted content, and external communication is not
complete inside it.

## Authentication

Every HTTP request carries a bearer token, compared in constant time. This
includes the MCP endpoint. A server with no configured token denies everything,
and the binary refuses to bind a non-loopback address without one.

The MCP transport also validates the inbound `Host` header, to prevent DNS
rebinding against a locally running server, so a deployment names the authority
it is reached by.

## The index

Query text reaches SQLite only as a MATCH expression built from quoted terms, so
FTS operators in user input cannot inject syntax.

## What is not a vulnerability

See `SECURITY.md`: the plaintext-over-a-private-network default, a provider's
accuracy, the absence of a rate limit, and `allow_private` reaching private
addresses are all by design.
