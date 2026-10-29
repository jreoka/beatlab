package sources

import "testing"

func TestCanonicalGenre(t *testing.T) {
	cases := []struct {
		in   any
		want string
	}{
		// semicolon-joined subject lists: first real genre wins
		{"live; guest sit-ins; Grateful Dead", ""},
		{"Jazz; Bop", "Jazz"},
		// reversed artist names are not genres
		{"Holiday, Billie", ""},
		{[]any{"Billie Holiday", "Jazz"}, "Jazz"},
		{[]any{"Classical"}, "Classical"},
		{"completely unrelated tag", ""},
		{nil, ""},
	}
	for _, c := range cases {
		if got := canonicalGenre(c.in); got != c.want {
			t.Errorf("canonicalGenre(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestParseYtTitle(t *testing.T) {
	artist, title := parseYtTitle("Ado - Usseewa (Official Music Video)", "Ado")
	if artist != "Ado" || title != "Usseewa" {
		t.Errorf("got %q / %q", artist, title)
	}
	artist, title = parseYtTitle("So What", "Miles Davis")
	if artist != "Miles Davis" || title != "So What" {
		t.Errorf("got %q / %q", artist, title)
	}
}
