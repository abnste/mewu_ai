// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Authentication;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Text;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class NetEaseMailTransportAuditTests
{
    [Theory]
    [InlineData("https://example.invalid/?sid=synthetic")]
    [InlineData("http://mail.163.com/")]
    [InlineData("https://mail.163.com.example.invalid/")]
    [InlineData("https://mail.163.com:8443/")]
    [InlineData("https://user@mail.163.com/")]
    [InlineData("https://127.0.0.1/")]
    [InlineData("//example.invalid/login")]
    [InlineData("javascript:alert(1)")]
    public void CookieBearingRedirectRejectsUntrustedDestinations(string destination)
        =>Assert.Throws<InvalidOperationException>(()=>NetEaseMailWebMcpService.ResolveAuthorizedRedirect(new Uri("https://mail.126.com/entry/login.jsp"),destination));

    [Fact]
    public void RelativeRedirectAndLegacySessionStayOnTheCorrectMailboxHost()
    {
        var uri=NetEaseMailWebMcpService.ResolveAuthorizedRedirect(new Uri("https://mail.126.com/entry/login.jsp"),"../js6/main.jsp?sid=synthetic");
        Assert.Equal("https://mail.126.com/js6/main.jsp?sid=synthetic",uri.AbsoluteUri);
        var session=new NetEaseWebSession("synthetic@126.com","synthetic",new Dictionary<string,string>(),DateTimeOffset.UnixEpoch);
        Assert.Equal("mail.126.com",NetEaseMailWebMcpService.GetMailboxHost(session));
        Assert.Equal("mail.yeah.net",NetEaseMailWebMcpService.GetMailboxHost(session with{MailHost="mail.yeah.net"}));
        Assert.Throws<InvalidDataException>(()=>NetEaseMailWebMcpService.GetMailboxHost(session with{MailHost="example.invalid"}));
    }

    [Theory]
    [InlineData("victim@example.test\r\nRCPT TO:<other@example.test>")]
    [InlineData("victim@example.test,other@example.test")]
    [InlineData("Display <victim@example.test>")]
    [InlineData("not an address")]
    public void SmtpEnvelopeRejectsCommandInjectionAndAmbiguousAddresses(string value)
    {
        Assert.False(NetEaseMailService.IsSingleAddress(value));
        Assert.Throws<InvalidOperationException>(()=>NetEaseMailService.ValidateEnvelope("sender@163.com",[value],[]));
    }

    [Fact]
    public void MimeBodyPreservesUnicodeAndCannotTerminateSmtpData()
    {
        var text=NetEaseMailService.BuildMessage(new AppSettings(),"sender@163.com",["recipient@example.test"],[],"主题","甲\n.\n乙");
        var encoded=text[(text.IndexOf("\r\n\r\n",StringComparison.Ordinal)+4)..];
        Assert.Equal("甲\r\n.\r\n乙",Encoding.UTF8.GetString(Convert.FromBase64String(encoded)));
        Assert.DoesNotContain("\r\n.\r\n",encoded);
    }

    [Fact]
    public async Task PostAcceptanceQuitFailureDoesNotInviteDuplicateSend()
    {
        using var stream=new MemoryStream();
        using var writer=new StreamWriter(stream){AutoFlush=true};
        using var reader=new StreamReader(new MemoryStream());
        await NetEaseMailService.TryQuitAsync(writer,reader,CancellationToken.None);
        using var canceled=new CancellationTokenSource();canceled.Cancel();
        await NetEaseMailService.TryQuitAsync(writer,reader,canceled.Token);
    }

    [Theory]
    [InlineData("",false)]
    [InlineData("not a reply\r\n",false)]
    [InlineData("250garbage\r\n",false)]
    [InlineData("250 accepted\r\n",true)]
    public async Task DataTerminatorNeedsAValidAcceptanceReply(string reply,bool accepted)
    {
        using var written=new MemoryStream();
        using var writer=new StreamWriter(written,new UTF8Encoding(false)){AutoFlush=true,NewLine="\r\n"};
        using var reader=new StreamReader(new MemoryStream(Encoding.ASCII.GetBytes(reply)));
        Assert.Equal(accepted,await NetEaseMailService.CompleteDataAsync(writer,reader,TestContext.Current.CancellationToken));
        Assert.Equal("\r\n.\r\n",Encoding.ASCII.GetString(written.ToArray()));
    }

    [Theory]
    [InlineData("450 retry later\r\n","SMTP 450")]
    [InlineData("550 rejected\r\n","SMTP 550")]
    public async Task ExplicitDataRejectionRemainsDefinite(string reply,string safeError)
    {
        using var written=new MemoryStream();
        using var writer=new StreamWriter(written){AutoFlush=true};
        using var reader=new StreamReader(new MemoryStream(Encoding.ASCII.GetBytes(reply)));
        var failure=await Record.ExceptionAsync(()=>NetEaseMailService.CompleteDataAsync(writer,reader,TestContext.Current.CancellationToken));
        Assert.NotNull(failure);Assert.Equal(safeError,failure.Message);
    }

    [Fact]
    public async Task CancellationBeforeDataTerminatorWritesNothing()
    {
        using var canceled=CancellationTokenSource.CreateLinkedTokenSource(TestContext.Current.CancellationToken);canceled.Cancel();
        using var written=new MemoryStream();
        using var writer=new StreamWriter(written,new UTF8Encoding(false)){AutoFlush=true};
        using var reader=new StreamReader(new MemoryStream());
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>NetEaseMailService.CompleteDataAsync(writer,reader,canceled.Token));
        Assert.Equal(0,written.Length);
    }

    [Fact]
    public async Task CancellationWhileWaitingForDataAcceptanceIsUnconfirmed()
    {
        using var canceled=CancellationTokenSource.CreateLinkedTokenSource(TestContext.Current.CancellationToken);
        using var written=new MemoryStream();
        using var writer=new StreamWriter(written,new UTF8Encoding(false)){AutoFlush=true,NewLine="\r\n"};
        using var reader=new StreamReader(new CancelReadStream(canceled));
        Assert.False(await NetEaseMailService.CompleteDataAsync(writer,reader,canceled.Token));
        Assert.Equal("\r\n.\r\n",Encoding.ASCII.GetString(written.ToArray()));
    }

    private sealed class CancelReadStream(CancellationTokenSource cancellation):MemoryStream
    {
        public override ValueTask<int> ReadAsync(Memory<byte> buffer,CancellationToken cancellationToken=default)
        {cancellation.Cancel();return ValueTask.FromCanceled<int>(cancellationToken);}
    }

    [Fact]
    public async Task SmtpTlsRejectsAnUntrustedCertificateWithoutSendingCredentials()
    {
        // Synthetic loopback TLS only: no mailbox, credential store or certificate installation.
        using var timeout=CancellationTokenSource.CreateLinkedTokenSource(TestContext.Current.CancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(10));
        using var rsa=RSA.Create(2048);
        var request=new CertificateRequest("CN=localhost",rsa,HashAlgorithmName.SHA256,RSASignaturePadding.Pkcs1);
        using var generated=request.CreateSelfSigned(DateTimeOffset.UtcNow.AddMinutes(-1),DateTimeOffset.UtcNow.AddMinutes(5));
        // Reimport the in-memory PFX so Windows Schannel can acquire its private
        // key. Nothing is installed in a certificate store or written by the test.
        using var certificate=X509CertificateLoader.LoadPkcs12(generated.Export(X509ContentType.Pfx),null);
        var listener=new TcpListener(IPAddress.Loopback,0);listener.Start();
        try
        {
            var clientFinished=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            Exception? serverFailure=null;
            var server=Task.Run(async()=>
            {
                using var connection=await listener.AcceptTcpClientAsync(timeout.Token);
                using var ssl=new SslStream(connection.GetStream(),false);
                try{await ssl.AuthenticateAsServerAsync(new SslServerAuthenticationOptions{ServerCertificate=certificate,EnabledSslProtocols=SslProtocols.Tls12},timeout.Token);}
                catch(Exception ex)when(ex is IOException or AuthenticationException){serverFailure=ex;}
                finally{await clientFinished.Task.WaitAsync(timeout.Token);}
            },TestContext.Current.CancellationToken);
            using var client=new TcpClient();
            await client.ConnectAsync(IPAddress.Loopback,((IPEndPoint)listener.LocalEndpoint).Port,timeout.Token);
            using var secure=NetEaseMailService.CreateSecureStream(client.GetStream());
            var policyErrors=SslPolicyErrors.None;var certificateWasChecked=false;
            Exception? failure;
            try
            {
                failure=await Record.ExceptionAsync(()=>secure.AuthenticateAsClientAsync(new SslClientAuthenticationOptions
                {
                    TargetHost="localhost",EnabledSslProtocols=SslProtocols.Tls12,
                    RemoteCertificateValidationCallback=(_,_,_,errors)=>
                    {certificateWasChecked=true;policyErrors=errors;return errors==SslPolicyErrors.None;}
                },timeout.Token));
            }
            finally{clientFinished.TrySetResult();}
            await server.WaitAsync(timeout.Token);
            Assert.True(certificateWasChecked,$"Certificate validation was not reached; client={failure?.GetType().Name}, server={serverFailure?.GetType().Name}: {serverFailure?.Message}");
            Assert.True(policyErrors.HasFlag(SslPolicyErrors.RemoteCertificateChainErrors));
            Assert.IsType<AuthenticationException>(failure);
            Assert.False(secure.IsAuthenticated);
        }
        finally{listener.Stop();}
    }
}
