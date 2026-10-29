package engine

import (
	"bytes"
	"image/png"
	"testing"
)

func TestGeneratedCoverIsRealPNG(t *testing.T) {
	a := renderGeneratedCover("al:grateful-dead|1972-03-25")
	if len(a) < 1000 {
		t.Fatalf("generated cover suspiciously small: %d bytes", len(a))
	}
	img, err := png.Decode(bytes.NewReader(a))
	if err != nil {
		t.Fatalf("not a PNG: %v", err)
	}
	if img.Bounds().Dx() != 400 {
		t.Fatalf("unexpected size: %v", img.Bounds())
	}
	b := renderGeneratedCover("al:miles-davis|kind-of-blue")
	if bytes.Equal(a, b) {
		t.Fatal("different seeds produced identical art")
	}
	if !bytes.Equal(a, renderGeneratedCover("al:grateful-dead|1972-03-25")) {
		t.Fatal("same seed produced different art — must be deterministic")
	}
}
