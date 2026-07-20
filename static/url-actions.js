"use strict";

function search_url(form) {
    const output = document.getElementById("results");
    output.innerHTML = "wait...";

    const params = {
        chan: form.chan.value,
        nick: form.nick.value,
        url: form.url.value,
        title: form.title.value,
    };
    const url = form.dataset.searchUrl + "?" + new URLSearchParams(params).toString();
    console.log("URL: " + url);

    const http = new XMLHttpRequest();
    http.open("GET", url, true);
    http.onreadystatechange = () => {
        if (http.readyState !== XMLHttpRequest.DONE) {
            return;
        }

        const html = http.responseText;
        console.log("Got HTML:\n" + html);
        output.innerHTML = "Search results from " + Date() + "<br>" + html + "<br>";
    };
    http.send();
}

function remove_url(id) {
    const output = document.getElementById("status_" + id);
    output.innerHTML = "removing...";

    const http = new XMLHttpRequest();
    http.open("GET", "/url2/search/remove_url?id=" + id, false);
    http.onreadystatechange = () => {
        output.innerHTML = http.responseText;
    };
    http.send();
}

function remove_meta(id) {
    const output = document.getElementById("status_" + id);
    output.innerHTML = "updating...";

    const http = new XMLHttpRequest();
    http.open("GET", "/url2/search/remove_meta?id=" + id, false);
    http.onreadystatechange = () => {
        output.innerHTML = http.responseText;
    };
    http.send();
}
