import { check, sleep } from 'k6';
import encoding from 'k6/encoding';
import grpc from 'k6/net/grpc';

const conf = {
    // baseURL: __ENV.BASE_URL || "grpcbin.test.k6.io:9001",
    url: 'host.docker.internal:50051',
    format: __ENV.FORMAT || 'M4A',
}


export let options = {
    stages: [
        { target: 10, duration: "30s" },
    ]
};

const binaryData = open('./1.wav', 'b');
const base64Data = encoding.b64encode(binaryData);
console.log('🧪 Data length (base64):', base64Data.length);
console.log('🧪 Test format:', conf.format);



const client = new grpc.Client();
client.load(['definitions'], 'audio_convert.proto');

function to_enum_format(formatStr) {
    switch (formatStr.toUpperCase()) {
        case 'MP3':
            return 1;
        case 'M4A':
            return 2;
            case 'ULAW':
            return 3;
        default:
            console.error('Unknown format:', formatStr);
            return 0;
    }
}

const formatEnum = to_enum_format(conf.format);

export default () => {
    console.log('connecting: ' + conf.url);
    client.connect(conf.url, {
        plaintext: true
    });

    const data = {
        format: formatEnum,
        metadata: ['olia=aaa'],
        data: base64Data,
    };
    const response = client.invoke('/audio_convert.v1.AudioConverter/Convert', data);

    check(response, {
        'status is OK': (r) => r && r.status === grpc.StatusOK,
    });

    if (response && response.status !== grpc.StatusOK) {
        console.log('Response:', response.message);
    }

    client.close();
    sleep(0.1);
};