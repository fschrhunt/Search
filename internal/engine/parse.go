package engine

import (
	"html"
	"regexp"
	"strings"
	"unicode"
)

var (
	tagRe   = regexp.MustCompile(`<[^>]+>`)
	spaceRe = regexp.MustCompile(`\s+`)
	hrefRe  = regexp.MustCompile(`href="([^"]*)"`)
)

// text strips tags, normalizes whitespace, and trims entities.
func text(s string) string {
	return strings.TrimSpace(spaceRe.ReplaceAllString(html.UnescapeString(tagRe.ReplaceAllString(s, " ")), " "))
}

// stripTags removes markup and unescapes entities, without collapsing to plain
// text (callers may want the spacing preserved).
func stripTags(s string) string {
	return strings.TrimSpace(html.UnescapeString(tagRe.ReplaceAllString(s, " ")))
}

// truncate cuts a string to n runes on a word boundary when possible.
func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	runes := []rune(s)
	if len(runes) <= n {
		return s
	}
	cut := string(runes[:n])
	if i := strings.LastIndexFunc(cut, unicode.IsSpace); i > n/2 {
		cut = cut[:i]
	}
	return strings.TrimSpace(cut) + "…"
}

// splitBlocks cuts html into segments between an opening marker and a closing
// marker, returning the segment bodies. It is the parse backbone for engines
// whose markup wraps each result in a tag carrying a class or attribute.
func splitBlocks(html, openMarker, closeMarker string) []string {
	var out []string
	rest := html
	for {
		i := strings.Index(rest, openMarker)
		if i < 0 {
			break
		}
		rest = rest[i:]
		j := strings.Index(rest[1:], closeMarker)
		if j < 0 {
			out = append(out, rest)
			break
		}
		out = append(out, rest[:j+1])
		rest = rest[j+1:]
	}
	return out
}

// firstAttr returns the value of the first attribute matching prefix, e.g.
// firstAttr(s, `href="`).
func firstAttr(s, prefix string) string {
	i := strings.Index(s, prefix)
	if i < 0 {
		return ""
	}
	rest := s[i+len(prefix):]
	j := strings.IndexAny(rest, `"`)
	if j < 0 {
		return ""
	}
	return rest[:j]
}

// matchTag returns the raw inner HTML of the first tag with the given name.
func matchTag(s, name string, classes ...string) string {
	open := "<" + name
	for {
		i := strings.Index(s, open)
		if i < 0 {
			return ""
		}
		close := ">"
		end := strings.Index(s[i:], close)
		if end < 0 {
			return ""
		}
		head := s[i : i+end+1]
		matches := true
		for _, c := range classes {
			if !strings.Contains(head, c) {
				matches = false
				break
			}
		}
		if matches {
			body := s[i+end+1:]
			stop := "</" + name + ">"
			j := strings.Index(body, stop)
			if j < 0 {
				return ""
			}
			return body[:j]
		}
		s = s[i+end+1:]
	}
}

// firstByClass returns the inner HTML of the first element whose opening tag
// contains class="name" (or class containing name), regardless of tag name.
func firstByClass(s, class string) string {
	marker := `class="`
	rest := s
	for {
		i := strings.Index(rest, marker)
		if i < 0 {
			return ""
		}
		start := i + len(marker)
		end := strings.Index(rest[start:], `"`)
		if end < 0 {
			return ""
		}
		value := rest[start : start+end]
		if !strings.Contains(value, class) {
			rest = rest[start+end:]
			continue
		}
		// Find the tag start and its matching close by scanning forward for the
		// next "<" at the same depth; this is a best-effort inner slice.
		tagStart := strings.LastIndex(rest[:i], "<")
		if tagStart < 0 {
			return ""
		}
		open := rest[tagStart:]
		gt := strings.Index(open, ">")
		if gt < 0 {
			return ""
		}
		body := open[gt+1:]
		trimmed := strings.TrimSpace(body)
		if strings.HasPrefix(trimmed, "<") {
			// Likely a nested container; take until the next block boundary.
			if j := strings.Index(body, "</div>"); j >= 0 {
				return body[:j]
			}
		}
		return body
	}
}

// firstByClassWithTag returns the inner HTML of the first element with the given
// tag name whose opening tag contains the class name.
func firstByClassWithTag(s, tag, class string) string {
	open := "<" + tag
	rest := s
	for {
		i := strings.Index(rest, open)
		if i < 0 {
			return ""
		}
		end := strings.Index(rest[i:], ">")
		if end < 0 {
			return ""
		}
		head := rest[i : i+end+1]
		if strings.Contains(head, class) {
			body := rest[i+end+1:]
			if j := strings.Index(body, "</"+tag+">"); j >= 0 {
				return body[:j]
			}
			return body
		}
		rest = rest[i+end+1:]
	}
}

// cleanURL trims tracking noise and rejects non-http(s) and internal links.
func cleanURL(raw string) string {
	raw = strings.TrimSpace(html.UnescapeString(raw))
	if raw == "" {
		return ""
	}
	for _, bad := range []string{"javascript:", "mailto:", "#", "data:"} {
		if strings.HasPrefix(raw, bad) {
			return ""
		}
	}
	return raw
}

// fallbackLinks extracts the first plausible external links when a structured
// parse finds nothing, so a markup change degrades rather than empties.
func fallbackLinks(html string, limit int, engine string) []Result {
	var out []Result
	seen := map[string]bool{}
	for _, m := range hrefRe.FindAllStringSubmatch(html, -1) {
		u := cleanURL(m[1])
		if u == "" || !strings.HasPrefix(u, "http") || seen[u] {
			continue
		}
		seen[u] = true
		out = append(out, Result{URL: u, Engine: engine})
		if len(out) >= limit {
			break
		}
	}
	return out
}
