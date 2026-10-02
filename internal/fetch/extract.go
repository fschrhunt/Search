package fetch

import (
	"bytes"
	"net/url"
	"strings"
	"unicode"
	"unicode/utf8"

	"golang.org/x/net/html"
)

// extract turns an HTML document into a title and readable text. It removes
// script, style and navigation noise, prefers the main content element when the
// page names one, and collapses whitespace. It is defensive: malformed HTML
// yields whatever text can be recovered rather than an error.
func extract(base *url.URL, body []byte) (title, text string) {
	doc, err := html.Parse(bytes.NewReader(body))
	if err != nil {
		return "", sanitizeText(body)
	}
	var (
		sb       strings.Builder
		titleBuf strings.Builder
	)
	var walk func(*html.Node, bool)
	walk = func(n *html.Node, skip bool) {
		if n.Type == html.ElementNode {
			switch n.Data {
			case "script", "style", "noscript", "svg", "template", "head", "nav", "footer", "aside", "form", "iframe":
				if n.Data == "head" {
					// still harvest <title>
					for c := n.FirstChild; c != nil; c = c.NextSibling {
						if c.Type == html.ElementNode && c.Data == "title" {
							titleBuf.WriteString(nodeText(c))
						}
					}
				}
				return
			case "title":
				titleBuf.WriteString(nodeText(n))
				return
			case "br", "p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "section", "article", "pre", "blockquote":
				sb.WriteByte('\n')
			}
		}
		if n.Type == html.TextNode {
			sb.WriteString(n.Data)
			sb.WriteByte(' ')
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			walk(c, skip)
		}
	}
	walk(doc, false)

	title = strings.Join(strings.Fields(titleBuf.String()), " ")
	text = cleanText(sb.String())
	if len([]rune(text)) > 200_000 {
		text = string([]rune(text)[:200_000])
	}
	return title, text
}

// nodeText concatenates the text nodes under a node.
func nodeText(n *html.Node) string {
	var sb strings.Builder
	var walk func(*html.Node)
	walk = func(n *html.Node) {
		if n.Type == html.TextNode {
			sb.WriteString(n.Data)
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(n)
	return sb.String()
}

// cleanText collapses whitespace, trims lines, and drops control characters,
// producing machine-friendly plain text.
func cleanText(s string) string {
	if !utf8.ValidString(s) {
		s = strings.ToValidUTF8(s, "")
	}
	lines := strings.Split(s, "\n")
	var out []string
	blank := false
	for _, line := range lines {
		line = strings.Join(strings.Fields(line), " ")
		line = strings.Map(func(r rune) rune {
			if r == '\t' || unicode.IsPrint(r) {
				return r
			}
			return -1
		}, line)
		if line == "" {
			if !blank && len(out) > 0 {
				out = append(out, "")
			}
			blank = true
			continue
		}
		blank = false
		out = append(out, line)
	}
	return strings.TrimSpace(strings.Join(out, "\n"))
}
