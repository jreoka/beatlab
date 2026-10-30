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

func TestParseYtTitleSlashConvention(t *testing.T) {
	artist, title := parseYtTitle("Reira Ushio / Harenohini 2026.04.04 CENTRAL FEST 2026", "汐れいら")
	if artist != "汐れいら" {
		t.Fatalf("artist should stay the channel, got %q", artist)
	}
	if title != "Harenohini 2026.04.04 CENTRAL FEST 2026" {
		t.Fatalf("title kept the artist prefix: %q", title)
	}
	// A latin-channel title with slashes is not the convention.
	_, title2 := parseYtTitle("Love / Hate", "Some Channel")
	if title2 != "Love / Hate" {
		t.Fatalf("latin slash title should be untouched, got %q", title2)
	}
}
