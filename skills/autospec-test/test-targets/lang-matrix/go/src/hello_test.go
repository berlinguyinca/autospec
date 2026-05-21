package hello

import "testing"

func TestHello(t *testing.T) {
	got := Hello("World")
	want := "Hello, World!"
	if got != want {
		t.Errorf("Hello() = %q, want %q", got, want)
	}
}
