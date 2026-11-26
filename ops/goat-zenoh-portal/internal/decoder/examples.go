// Per-language "subscribe and decode" snippets templated from the
// descriptor + topic key. Consumed by the schema viewer's "Examples"
// section so a vendor can copy-paste a working starting point in
// Rust / Go / Python / TypeScript.
//
// These are starting points, not paste-ready production code. They
// assume the consumer has run their language's protoc plugin against
// the .proto and has the generated types on their import path.

package decoder

import (
	"bytes"
	"text/template"
)

// ExampleParams carries everything the per-language templates need.
type ExampleParams struct {
	// TopicKey — the full Zenoh key the topic is published on.
	TopicKey string
	// Package — the proto package, e.g. "caracal.power_demo.v1".
	Package string
	// MessageFullName — e.g. "caracal.power_demo.v1.PowerState".
	MessageFullName string
	// MessageName — the bare type name, e.g. "PowerState".
	MessageName string
	// SampleField — a representative scalar field name to print in
	// the example (auto-picked: first singular string / int / double).
	SampleField string
	// SampleFieldJSON — the camelCase JSON name protobuf emits for
	// SampleField (e.g. "siteId" for "site_id"); used in the TS snippet.
	SampleFieldJSON string
	// ProtoFile — the .proto file name (e.g.
	// "caracal_power_demo_site_jb_v1.proto"). Templates use this to
	// hint where the generated module comes from.
	ProtoFile string
}

// CodeExample is one language's snippet plus enough metadata for the
// frontend to label the tab + pick the highlight.js language tag.
//
// Source distinguishes org-provided overrides from portal-templated
// autogen so the viewer can badge them differently. Today the renderer
// only emits "autogen"; org-override storage adds the "org" path.
// (ADR 1052 Phase 3: the provenance value flipped "vendor"→"org".)
type CodeExample struct {
	Language     string `json:"language"`
	DisplayName  string `json:"display_name"`
	HighlightTag string `json:"highlight_tag"`
	Code         string `json:"code"`
	Source       string `json:"source"` // "autogen" | "org"
}

// RenderCodeExamples returns one snippet per supported language, each
// templated against the params. Returns nil + nil error when the
// params don't provide enough to template (e.g. empty MessageName) —
// the viewer just hides the section in that case.
func RenderCodeExamples(p ExampleParams) []CodeExample {
	if p.MessageName == "" || p.TopicKey == "" {
		return nil
	}
	if p.SampleField == "" {
		p.SampleField = "ts_ms"
		p.SampleFieldJSON = "tsMs"
	}
	out := make([]CodeExample, 0, 4)
	for _, t := range exampleTemplates {
		var buf bytes.Buffer
		if err := t.tmpl.Execute(&buf, p); err != nil {
			continue
		}
		out = append(out, CodeExample{
			Language:     t.lang,
			DisplayName:  t.display,
			HighlightTag: t.hljs,
			Code:         buf.String(),
			Source:       "autogen",
		})
	}
	return out
}

type exampleTemplate struct {
	lang    string
	display string
	hljs    string
	tmpl    *template.Template
}

var helpers = template.FuncMap{
	"pascalCase":       pascalCase,
	"snakeCase":        snakeCase,
	"kebabCase":        kebabCase,
	"slashPath":        slashPath,
	"underscoreModule": underscoreModule,
}

// mustTmpl is a Must() variant that registers helpers before parse so
// the var-initialized template literals below can call them.
func mustTmpl(name, body string) *template.Template {
	return template.Must(template.New(name).Funcs(helpers).Parse(body))
}

var exampleTemplates = []exampleTemplate{
	{
		lang:    "rust",
		display: "Rust",
		hljs:    "rust",
		tmpl: mustTmpl("rust", `// Cargo.toml dependencies:
//   prost            = "0.13"
//   zenoh            = "1.9"
//   tokio            = { version = "1", features = ["full"] }
//   anyhow           = "1"
//
// Generate Rust types from the .proto via a build.rs that calls
// prost-build on {{.ProtoFile}}; the include! pulls them in.

use prost::Message;
use zenoh::Config;

include!(concat!(env!("OUT_DIR"), "/{{.Package}}.rs"));

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let session = zenoh::open(Config::default()).await?;
    let sub = session
        .declare_subscriber("{{.TopicKey}}")
        .await?;
    while let Ok(sample) = sub.recv_async().await {
        let bytes = sample.payload().to_bytes();
        let msg = {{.MessageName}}::decode(bytes.as_ref())?;
        println!("{{.SampleField}}={:?}", msg.{{.SampleField}});
    }
    Ok(())
}
`),
	},
	{
		lang:    "go",
		display: "Go",
		hljs:    "go",
		tmpl: mustTmpl("go", `// go.mod:
//   require google.golang.org/protobuf v1.34.0
//   require github.com/eclipse-zenoh/zenoh-go v1.9.0
//
// Generate Go types from the .proto via:
//   protoc --go_out=. --go_opt=paths=source_relative {{.ProtoFile}}

package main

import (
	"context"
	"fmt"

	zenoh "github.com/eclipse-zenoh/zenoh-go"
	"google.golang.org/protobuf/proto"

	pb "your/import/path/{{.Package}}"
)

func main() {
	cfg := zenoh.NewConfig()
	z, err := zenoh.Open(cfg)
	if err != nil {
		panic(err)
	}
	defer z.Close()

	sub, err := z.Subscribe(context.Background(), "{{.TopicKey}}")
	if err != nil {
		panic(err)
	}
	for sample := range sub.RecvC() {
		var msg pb.{{.MessageName}}
		if err := proto.Unmarshal(sample.Payload, &msg); err != nil {
			fmt.Println("decode error:", err)
			continue
		}
		fmt.Printf("{{.SampleField}}=%v\n", msg.Get{{.SampleField | pascalCase}}())
	}
}
`),
	},
	{
		lang:    "python",
		display: "Python",
		hljs:    "python",
		tmpl: mustTmpl("python", `# requirements.txt:
#   protobuf>=5.27
#   eclipse-zenoh>=1.9
#
# Generate Python types from the .proto via:
#   protoc --python_out=. {{.ProtoFile}}
# That produces a sibling *_pb2.py module you import from.

import zenoh

# Replace the import below with the actual generated module path.
from {{.Package | underscoreModule}} import {{.MessageName | snakeCase}}_pb2


def on_sample(sample: zenoh.Sample) -> None:
    msg = {{.MessageName | snakeCase}}_pb2.{{.MessageName}}()
    msg.ParseFromString(bytes(sample.payload))
    print(f"{{.SampleField}}={getattr(msg, '{{.SampleField}}')}")


def main() -> None:
    config = zenoh.Config()
    with zenoh.open(config) as session:
        _sub = session.declare_subscriber("{{.TopicKey}}", on_sample)
        try:
            input("subscribing — press Enter to exit\n")
        except (EOFError, KeyboardInterrupt):
            pass


if __name__ == "__main__":
    main()
`),
	},
	{
		lang:    "typescript",
		display: "TypeScript",
		hljs:    "typescript",
		tmpl: mustTmpl("ts", `// package.json deps:
//   "@eclipse-zenoh/zenoh-ts": "^1.10",
//   "@bufbuild/protobuf":      "^2.0"   // OR your generator of choice
//
// Generate TS types from the .proto via buf generate or protoc-gen-ts;
// the import below assumes the generated file lives under ./generated/.

import { open, Config } from "@eclipse-zenoh/zenoh-ts";
import { {{.MessageName}} } from "./generated/{{.Package | slashPath}}/{{.MessageName | kebabCase}}_pb";

async function main(): Promise<void> {
  const session = await open(new Config());
  const sub = await session.declareSubscriber(
    "{{.TopicKey}}",
    (sample) => {
      const msg = {{.MessageName}}.fromBinary(new Uint8Array(sample.payload));
      console.log(`+"`"+`{{.SampleField}}=${msg.{{.SampleFieldJSON}}}`+"`"+`);
    },
  );
  // Keep the process alive — Ctrl-C to exit.
  process.on("SIGINT", () => sub.undeclare());
}

main().catch(console.error);
`),
	},
}

// pascalCase: "ts_ms" → "TsMs"; "site_id" → "SiteId".
func pascalCase(s string) string {
	var out []byte
	upper := true
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c == '_' || c == '-' {
			upper = true
			continue
		}
		if upper && c >= 'a' && c <= 'z' {
			c -= 'a' - 'A'
		}
		out = append(out, c)
		upper = false
	}
	return string(out)
}

// snakeCase: "PowerState" → "power_state"; "ts_ms" → "ts_ms".
func snakeCase(s string) string {
	var out []byte
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c >= 'A' && c <= 'Z' {
			if i > 0 && (out[len(out)-1] != '_') {
				out = append(out, '_')
			}
			c += 'a' - 'A'
		}
		out = append(out, c)
	}
	return string(out)
}

// kebabCase: "PowerState" → "power-state".
func kebabCase(s string) string {
	snake := snakeCase(s)
	out := make([]byte, len(snake))
	for i := 0; i < len(snake); i++ {
		if snake[i] == '_' {
			out[i] = '-'
		} else {
			out[i] = snake[i]
		}
	}
	return string(out)
}

// slashPath: "caracal.power_demo.v1" → "caracal/power_demo/v1".
func slashPath(s string) string {
	out := make([]byte, len(s))
	for i := 0; i < len(s); i++ {
		if s[i] == '.' {
			out[i] = '/'
		} else {
			out[i] = s[i]
		}
	}
	return string(out)
}

// underscoreModule: "caracal.power_demo.v1" → "caracal.power_demo.v1"
// (already valid Python dotted-module path; identity here).
func underscoreModule(s string) string { return s }
