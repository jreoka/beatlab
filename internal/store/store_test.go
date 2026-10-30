package store

import "testing"

// Non-latin names must not all collapse into one "unknown"
// artist/album bucket.
func TestSlugNonLatinNamesStayDistinct(t *testing.T) {
	a := slug("ヨルシカ")
	b := slug("米津玄師")
	if a == "unknown" || b == "unknown" {
		t.Fatalf("non-latin names slugged to unknown: %q %q", a, b)
	}
	if a == b {
		t.Fatalf("distinct non-latin names collided: %q", a)
	}
	if slug("ヨルシカ") != a {
		t.Fatal("slug must be stable across calls")
	}
	ta := Track{Artist: "ヨルシカ"}
	tb := Track{Artist: "米津玄師"}
	if ta.ArtistID() == tb.ArtistID() {
		t.Fatal("artist ids collided for distinct non-latin artists")
	}
	if slug("") != "unknown" || slug("Reira Ushio") != "reira-ushio" {
		t.Fatal("latin/empty slug behavior changed")
	}
}
