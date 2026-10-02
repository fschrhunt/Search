package mcp

import (
	"context"
	"strings"
	"testing"

	sdkmcp "github.com/modelcontextprotocol/go-sdk/mcp"
)

// TestMCPServerListsBothTools pins the agent-facing contract: the server offers
// web_search and web_fetch with descriptions a model can act on.
func TestMCPServerListsBothTools(t *testing.T) {
	srv := New(Deps{Version: "test"})
	client := sdkmcp.NewClient(&sdkmcp.Implementation{Name: "test", Version: "1"}, nil)
	ctx := context.Background()

	t1, t2 := sdkmcp.NewInMemoryTransports()
	ss, err := srv.Connect(ctx, t1, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer ss.Close()
	cs, err := client.Connect(ctx, t2, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer cs.Close()

	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	names := map[string]string{}
	for _, tool := range tools.Tools {
		names[tool.Name] = tool.Description
	}
	for _, want := range []string{"web_search", "web_fetch"} {
		desc, ok := names[want]
		if !ok {
			t.Fatalf("tool %q not offered", want)
		}
		if strings.TrimSpace(desc) == "" {
			t.Fatalf("tool %q has no description", want)
		}
	}
}
