package server

import "testing"

func TestBinaryTargetRejectsTraversal(t *testing.T) {
	for _, pair := range [][2]string{{"../etc", "passwd"}, {"linux/../..", "amd64"}, {"linux", "../../etc/passwd"}, {"linux", "arm64/.."}, {"", ""}} {
		if validBinaryTarget(pair[0], pair[1]) {
			t.Errorf("accepted untrusted binary target %q", pair)
		}
	}
	for _, system := range []string{"linux", "darwin", "windows"} {
		for _, arch := range []string{"amd64", "arm64"} {
			if !validBinaryTarget(system, arch) {
				t.Errorf("refused supported target %s/%s", system, arch)
			}
		}
	}
}
