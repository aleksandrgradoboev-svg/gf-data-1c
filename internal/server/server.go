// Пакет server собирает MCP-сервер: инструкции, набор инструментов, транспорт.
package server

import (
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/tools"
)

// Version — версия продукта. Расширение в базе сверяет её со своей.
const Version = "0.1.0"

// Options — то, что задаётся при запуске.
type Options struct {
	RegistryPath string
	Timeout      time.Duration
}

// New собирает сервер со всеми инструментами.
func New(opts Options) *mcp.Server {
	set := &tools.Set{RegistryPath: opts.RegistryPath, Timeout: opts.Timeout, Version: Version}

	srv := mcp.NewServer(
		&mcp.Implementation{Name: "gt-data-1c", Version: Version},
		&mcp.ServerOptions{Instructions: instructions},
	)

	// Порядок регистрации — порядок знакомства агента с базой: где мы работаем,
	// жив ли канал, что за конфигурация, из чего она состоит, и только потом данные.
	mcp.AddTool(srv, tools.BasesTool(), set.Bases)
	mcp.AddTool(srv, tools.ProbeTool(), set.Probe)
	mcp.AddTool(srv, tools.BaseInfoTool(), set.BaseInfo)
	mcp.AddTool(srv, tools.MetadataTool(), set.Metadata)
	mcp.AddTool(srv, tools.ObjectTool(), set.Object)
	mcp.AddTool(srv, tools.QueryCheckTool(), set.QueryCheck)
	mcp.AddTool(srv, tools.QueryParseTool(), set.QueryParse)
	mcp.AddTool(srv, tools.QueryBuildTool(), set.QueryBuild)
	mcp.AddTool(srv, tools.QueryTool(), set.Query)
	mcp.AddTool(srv, tools.CountTool(), set.Count)
	mcp.AddTool(srv, tools.RegisterTool(), set.Register)
	mcp.AddTool(srv, tools.SliceTool(), set.Slice)
	mcp.AddTool(srv, tools.AccountsTool(), set.Accounts)
	mcp.AddTool(srv, tools.ExportTool(), set.Export)
	mcp.AddTool(srv, tools.SyntaxTool(), set.Syntax)
	mcp.AddTool(srv, tools.EventLogTool(), set.EventLog)

	return srv
}

// instructions — то, что сервер говорит агенту при подключении.
//
// Этот текст влияет на поведение агента сильнее описаний отдельных инструментов:
// именно здесь сказано, что отказ нельзя читать как отсутствие данных.
const instructions = `Сервер читает конфигурацию и данные информационных баз 1С:Предприятие.
Читай ответы буквально: чего в ответе нет, того сервер не утверждал.

Баз несколько. Их список отдаёт bases (action=list), нужная называется параметром base.
base ОБЯЗАТЕЛЕН у каждого инструмента данных, и базы по умолчанию у сервера нет: вызов
без base не выполняется, а отвечает отказом с перечнем баз. Незнакомое имя базы тоже
даёт отказ с перечнем известных, а не пустой результат.

Единственное исключение — probe: там пустой base значит «проверить все базы», и ответ
называет каждую поимённо.

Отсюда же читай отказ «объект не найден»: он говорит про НАЗВАННУЮ базу, а не про 1С
вообще. Если объект есть в соседней базе реестра, отказ скажет и это. Перебирать имена
объекта, пока какое-нибудь не найдётся, — неверный ход: сначала проверь, та ли база.

Ответ, начинающийся словом ОТКАЗ, говорит о ВЫЗОВЕ, а не о содержимом базы: запрошенное
не выполнено. Прежде чем считать пустой ответ фактом, проверь канал инструментом probe —
он различает погашенный веб-сервер, неустановленное расширение и отказ прав.`
