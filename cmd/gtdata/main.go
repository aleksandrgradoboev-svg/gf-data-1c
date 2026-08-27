// Команда gtdata — MCP-сервер доступа к данным информационных баз 1С:Предприятие.
//
// Запускается агентом как дочерний процесс и общается по stdio. Регистрация у агента
// сводится к пути этого бинарника: реестр баз лежит в профиле пользователя, флаги
// нужны только для нештатных случаев.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/aleksandrgradoboev-svg/gt-data-1c/internal/channel"
	"github.com/aleksandrgradoboev-svg/gt-data-1c/internal/installer"
	"github.com/aleksandrgradoboev-svg/gt-data-1c/internal/journal"
	"github.com/aleksandrgradoboev-svg/gt-data-1c/internal/registry"
	"github.com/aleksandrgradoboev-svg/gt-data-1c/internal/server"
)

func main() {
	var (
		registryPath = flag.String("registry", "", "путь к реестру баз (по умолчанию — профиль пользователя)")
		timeout      = flag.Duration("timeout", channel.DefaultTimeout, "таймаут обращения к базе")
		logPath      = flag.String("log", "", "вести журнал сервера в этот файл (пусто — не вести; "+
			"значение auto — путь по умолчанию)")
		showVersion = flag.Bool("version", false, "напечатать версию и выйти")

		httpAddr = flag.String("http", "", "сетевой режим: слушать этот адрес вместо stdio "+
			"(например 127.0.0.1:9077 или просто «auto» для адреса по умолчанию). Один процесс "+
			"обслуживает несколько сессий агента сразу")
		httpToken = flag.String("token", "", "сетевой режим: требовать заголовок "+
			"Authorization: Bearer <токен>")

		install = flag.String("install", "", "установить расширение в базу: путь к файловой базе "+
			"или строка сервер\\база вместе с -server")
		// Переменная называется не server: так зовётся пакет, и тень над ним ломает сборку.
		serverBase = flag.Bool("server", false, "значение -install — строка подключения к серверной базе")
		dbUser     = flag.String("db-user", "", "пользователь базы для конфигуратора (режим установки)")
		dbPassword = flag.String("db-password", "", "пароль пользователя базы (режим установки)")
		platform   = flag.String("platform", "", "путь к 1cv8.exe (по умолчанию ищется старшая версия)")
	)
	flag.Parse()

	if *install != "" {
		err := installer.Install(installer.Options{
			Base: *install, Server: *serverBase,
			User: *dbUser, Password: *dbPassword, Platform: *platform,
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "gt-data-1c: расширение не установлено: %v\n", err)
			os.Exit(1)
		}
		fmt.Printf("Расширение %s установлено в базу %s.\n", installer.ExtensionName, *install)
		fmt.Println("Дальше: опубликуйте базу на веб-сервере и зарегистрируйте её " +
			"инструментом bases (action=add).")
		return
	}

	if *showVersion {
		fmt.Printf("gt-data-1c %s\nреестр баз: %s\nжурнал по умолчанию: %s\n",
			server.Version, registryDefault(*registryPath), journal.DefaultPath())
		return
	}

	if *logPath != "" {
		path := *logPath
		if path == "auto" {
			path = journal.DefaultPath()
		}
		if err := journal.Open(path); err != nil {
			// Журнал — удобство, а не условие работы: сказать и продолжить.
			fmt.Fprintf(os.Stderr, "gt-data-1c: журнал не открыт (%v), работаю без него\n", err)
		}
		defer journal.Close()
	}

	options := server.Options{RegistryPath: *registryPath, Timeout: *timeout}

	// Чтение реестра при старте нужно ради побочного действия: пароли, оставшиеся
	// открытыми (вписанные руками или от прежней версии), защищаются сразу, а не при
	// ближайшем изменении реестра — которого может не случиться месяцами.
	if _, err := registry.Load(*registryPath); err != nil {
		fmt.Fprintf(os.Stderr, "gt-data-1c: реестр баз не прочитан: %v\n", err)
	}

	if *httpAddr != "" {
		addr := *httpAddr
		if addr == "auto" {
			addr = server.DefaultHTTPAddr
		}
		err := server.ServeHTTP(options, server.HTTPOptions{Addr: addr, Token: *httpToken})
		if err != nil {
			fmt.Fprintf(os.Stderr, "gt-data-1c: сетевой режим остановлен: %v\n", err)
			os.Exit(1)
		}
		return
	}

	srv := server.New(options)

	if err := srv.Run(context.Background(), &mcp.StdioTransport{}); err != nil {
		fmt.Fprintf(os.Stderr, "gt-data-1c: сервер остановлен: %v\n", err)
		os.Exit(1)
	}
}

func registryDefault(path string) string {
	if path != "" {
		return path
	}
	return registry.DefaultPath()
}
