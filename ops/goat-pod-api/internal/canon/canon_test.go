package canon

import "testing"

func TestFromJSON(t *testing.T) {
	cases := map[string]string{
		`{ "b": 1, "a": [3, 2, {"z": null, "y": true}] }`: `{"a":[3,2,{"y":true,"z":null}],"b":1}`,
		`{"n": 12345678901234567890}`:                     `{"n":12345678901234567890}`,
		`{"s": "a<b>&c é"}`:                               `{"s":"a<b>&c é"}`,
	}
	for in, want := range cases {
		got, err := FromJSON([]byte(in))
		if err != nil {
			t.Fatalf("%s: %v", in, err)
		}
		if string(got) != want {
			t.Errorf("FromJSON(%s) = %s, want %s", in, got, want)
		}
	}
}

func TestTrailingData(t *testing.T) {
	if _, err := FromJSON([]byte(`{} {}`)); err == nil {
		t.Fatal("trailing document accepted")
	}
}

func TestWithoutKey(t *testing.T) {
	body, sig, err := WithoutKey([]byte(`{"signature":"abc","x":1}`), "signature")
	if err != nil {
		t.Fatal(err)
	}
	if sig != "abc" || string(body) != `{"x":1}` {
		t.Fatalf("got %q %s", sig, body)
	}
}
