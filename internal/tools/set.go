package tools

import (
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/channel"
	"github.com/greentech/gt-data-1c/internal/refusal"
	"github.com/greentech/gt-data-1c/internal/registry"
)

// Set — набор инструментов, разделяющих общее состояние: путь реестра и таймаут канала.
// Реестр читается на каждый вызов, а не кэшируется: его правят снаружи (в том числе
// инструментом bases), и агент не должен видеть устаревший список.
type Set struct {
	RegistryPath string
	Timeout      time.Duration
	// Version — версия сервера. Нужна пробе: расширение старше сервера отвечает не
	// ошибкой, а пустотой в новых методах, и это неотличимо от отсутствия данных.
	Version string
}

func (s *Set) registry() (*registry.Registry, error) {
	return registry.Load(s.RegistryPath)
}

// channelFor открывает канал к названной базе, разрешая имя по реестру.
func (s *Set) channelFor(name string) (*channel.Client, error) {
	reg, err := s.registry()
	if err != nil {
		return nil, err
	}
	base, err := reg.Resolve(name)
	if err != nil {
		return nil, err
	}
	return channel.New(base, s.Timeout), nil
}

// text — обычный текстовый ответ инструмента.
func text(s string) *mcp.CallToolResult {
	return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: s}}}
}

var _ = refusal.Internal
