package config

import "testing"

// TestValidateRequiresTokenOnAllInterfaces pins that ":port" — which binds every
// interface — is not mistaken for loopback, so it demands a token.
func TestValidateRequiresTokenOnAllInterfaces(t *testing.T) {
	cases := []struct {
		addr    string
		token   string
		wantErr bool
	}{
		{":8642", "", true},          // all interfaces, no token
		{"0.0.0.0:8642", "", true},   // all v4, no token
		{"[::]:8642", "", true},      // all v6, no token
		{"100.1.2.3:8642", "", true}, // tailnet address, no token
		{"127.0.0.1:8642", "", false},
		{"localhost:8642", "", false},
		{"[::1]:8642", "", false},
		{":8642", "a-strong-enough-token", false},
		{"0.0.0.0:8642", "short", true}, // non-loopback with a too-short token
	}
	for _, tc := range cases {
		c := &Config{Addr: tc.addr, Token: tc.token}
		err := c.Validate()
		if tc.wantErr && err == nil {
			t.Errorf("Validate(%q, token=%q) = nil, want error", tc.addr, tc.token)
		}
		if !tc.wantErr && err != nil {
			t.Errorf("Validate(%q, token=%q) = %v, want nil", tc.addr, tc.token, err)
		}
	}
}

// TestValidateRejectsMalformedAddr pins that a bad address fails clearly.
func TestValidateRejectsMalformedAddr(t *testing.T) {
	c := &Config{Addr: "not-an-address", Token: "a-strong-enough-token"}
	if err := c.Validate(); err == nil {
		t.Fatal("expected an error for a malformed addr")
	}
}
